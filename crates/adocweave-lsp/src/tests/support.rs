//! Shared fixtures for service and protocol tests.

use super::*;

pub(super) fn typed<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("valid LSP value")
}

pub(super) fn uri(value: &str) -> lsp::Url {
    value.parse().expect("valid URI")
}

pub(super) fn initialize_with_params(
    service: &mut Session,
    params: lsp::InitializeParams,
) -> lsp::InitializeResult {
    service.initialize(&params)
}

pub(super) async fn write_message(output: &mut (impl AsyncWriteExt + Unpin), message: &Value) {
    let body = serde_json::to_vec(message).expect("serialize");
    output
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await
        .expect("header");
    output.write_all(&body).await.expect("body");
    output.flush().await.expect("flush");
}

pub(super) async fn read_message(
    input: &mut BufReader<impl tokio::io::AsyncRead + Unpin>,
) -> Value {
    let mut content_length = None;
    loop {
        let mut header = String::new();
        input.read_line(&mut header).await.expect("header");
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>().expect("length"));
        }
    }
    let mut body = vec![0; content_length.expect("content length")];
    input.read_exact(&mut body).await.expect("body");
    serde_json::from_slice(&body).expect("json")
}

pub(super) fn initialize(service: &mut Session, encodings: &[&str]) -> lsp::InitializeResult {
    let params = typed(json!({
        "processId": null,
        "rootUri": null,
        "capabilities": full_capabilities(encodings)
    }));
    initialize_with_params(service, params)
}

pub(super) fn full_capabilities(encodings: &[&str]) -> Value {
    json!({
        "general": {"positionEncodings": encodings},
        "workspace": {
            "workspaceEdit": {"documentChanges": true},
            "workspaceFolders": true,
            "didChangeWatchedFiles": {"dynamicRegistration": true}
        },
        "textDocument": {
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": {"valueSet": ["quickfix"]}
                },
                "isPreferredSupport": true
            },
            "documentLink": {"tooltipSupport": true},
            "publishDiagnostics": {"versionSupport": true},
            "rename": {"prepareSupport": true},
            "semanticTokens": {
                "requests": {"full": true},
                "tokenTypes": ["string", "variable"],
                "tokenModifiers": [],
                "formats": ["relative"]
            }
        }
    })
}

pub(super) fn open(service: &mut Session, uri: &str, version: i32, text: &str) {
    let jobs = service.begin_open(typed(json!({
        "textDocument": {
            "uri": uri,
            "languageId": "asciidoc",
            "version": version,
            "text": text
        }
    })));
    for job in jobs {
        adopt(service, job);
    }
}

pub(super) fn all_code_actions(
    service: &Session,
    document_uri: &lsp::Url,
) -> Result<Option<Vec<lsp::CodeActionOrCommand>>, String> {
    service.code_actions(
        document_uri,
        lsp::Range::new(
            lsp::Position::new(0, 0),
            lsp::Position::new(u32::MAX, u32::MAX),
        ),
        &lsp::CodeActionContext {
            diagnostics: Vec::new(),
            only: None,
            trigger_kind: None,
        },
    )
}

pub(super) fn change(
    service: &mut Session,
    uri: &str,
    version: i32,
    changes: Value,
) -> Result<bool, String> {
    let jobs = service.begin_change(typed(json!({
        "textDocument": {"uri": uri, "version": version},
        "contentChanges": changes
    })))?;
    if jobs.is_empty() {
        return Ok(false);
    }
    for job in jobs {
        adopt(service, job);
    }
    Ok(true)
}

/// Runs one job the way the event loop and its worker do, then adopts it.
pub(super) fn adopt(service: &mut Session, job: ProjectAnalysisSnapshot) {
    let _ = adopt_completion(service, process_project_snapshot(job));
}

pub(super) fn adopt_completion(
    service: &mut Session,
    completion: crate::service::ProjectAnalysisCompletion,
) -> Vec<String> {
    let next = service.project_processing_completed(completion);
    let next = match next {
        crate::service::ProjectAnalysisAction::Validate(completion) => {
            service.complete_analysis(validate(*completion))
        }
        next => next,
    };
    adopt_next(service, next)
}

pub(super) fn adopt_next(
    service: &mut Session,
    action: crate::service::ProjectAnalysisAction,
) -> Vec<String> {
    match action {
        crate::service::ProjectAnalysisAction::Publish {
            diagnostic_uris, ..
        } => diagnostic_uris,
        crate::service::ProjectAnalysisAction::Retry(snapshot) => {
            adopt_completion(service, process_project_snapshot(snapshot))
        }
        crate::service::ProjectAnalysisAction::Validate(completion) => {
            let next = service.complete_analysis(validate(*completion));
            adopt_next(service, next)
        }
        crate::service::ProjectAnalysisAction::Ignore => panic!("analysis was not adopted"),
    }
}

pub(super) fn process_project_snapshot(
    mut job: ProjectAnalysisSnapshot,
) -> crate::service::ProjectAnalysisCompletion {
    let (outcome, source_index, observation_access) = match job.prepared_request.take() {
        Some(project) => (
            crate::service::ProjectAnalysisOutcome::Processed(adocweave_project::process(
                project.request,
                job.cancellation.as_ref(),
            )),
            project.source_index,
            Some(project.observation_access),
        ),
        None => (
            crate::service::ProjectAnalysisOutcome::Rejected(
                job.project_problem.clone().expect("project problem"),
            ),
            crate::state::ProjectSourceIndex::default(),
            None,
        ),
    };
    crate::service::ProjectAnalysisCompletion {
        snapshot: job,
        outcome,
        source_index,
        observation_access,
        observations_are_current: None,
    }
}

pub(super) fn validate(
    mut completion: crate::service::ProjectAnalysisCompletion,
) -> crate::service::ProjectAnalysisCompletion {
    let access = completion
        .observation_access
        .as_ref()
        .expect("observation access");
    let crate::service::ProjectAnalysisOutcome::Processed(result) = &completion.outcome else {
        unreachable!("only processed results require validation");
    };
    completion.observations_are_current = Some(crate::service::project_observations_are_current(
        result,
        access,
        &adocweave_core::NeverCancel,
    ));
    completion
}

pub(super) fn apply_edits(source: &str, edits: &[lsp::TextEdit]) -> String {
    use adocweave_core::text::{
        Position, PositionEncoding as CorePositionEncoding, SourceDocument,
    };

    let index = SourceDocument::new(source).expect("line index");
    let mut byte_edits = edits
        .iter()
        .map(|edit| {
            let position = |position: lsp::Position| Position {
                line: position.line,
                character: position.character,
            };
            let start = index
                .position_to_offset(position(edit.range.start), CorePositionEncoding::Utf16)
                .expect("start")
                .to_usize();
            let end = index
                .position_to_offset(position(edit.range.end), CorePositionEncoding::Utf16)
                .expect("end")
                .to_usize();
            (start, end, edit.new_text.clone())
        })
        .collect::<Vec<_>>();
    byte_edits.sort_by_key(|(start, end, _)| (*start, *end));
    let mut output = source.to_owned();
    for (start, end, replacement) in byte_edits.into_iter().rev() {
        output.replace_range(start..end, &replacement);
    }
    output
}

pub(super) fn open_reference_workspace(service: &mut Session) {
    open(
        service,
        "file:///a.adoc",
        1,
        "[[target]]\n== Target\n\nSee <<target>> and xref:b.adoc#other[B].\nhttps://example.com[Site]\n",
    );
    open(
        service,
        "file:///b.adoc",
        1,
        "[[other]]\n== Other\n\nxref:a.adoc#target[A]\n",
    );
}
