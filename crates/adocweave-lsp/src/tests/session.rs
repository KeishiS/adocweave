use adocweave::CancellationCheck;

use super::*;

fn initialize_params(roots: &[&str]) -> lsp::InitializeParams {
    typed(json!({
        "processId": null,
        "capabilities": full_capabilities(&["utf-8", "utf-16"]),
        "workspaceFolders": roots
            .iter()
            .enumerate()
            .map(|(index, root)| json!({
                "uri": root,
                "name": format!("root-{index}")
            }))
            .collect::<Vec<_>>()
    }))
}

#[test]
fn one_session_owns_connection_lifecycle_and_cancellation() {
    let mut session = Session::default();
    assert_eq!(session.input_revision(), 0);

    session.initialize(&initialize_params(&[]));
    assert_eq!(session.input_revision(), 1);

    let jobs = session.begin_open(typed(json!({
        "textDocument": {
            "uri": "file:///guide.adoc",
            "languageId": "asciidoc",
            "version": 1,
            "text": "= Guide\n"
        }
    })));
    let cancellation = jobs[0].cancellation.clone();
    assert!(!cancellation.is_cancelled());

    session.shutdown();
    assert!(cancellation.is_cancelled());
}

#[test]
fn one_project_request_captures_primary_and_open_include_overlays() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    open(
        &mut session,
        "file:///book/part.adoc",
        1,
        "included overlay\n",
    );
    let jobs = session.begin_open(typed(json!({
        "textDocument": {
            "uri": "file:///book/root.adoc",
            "languageId": "asciidoc",
            "version": 1,
            "text": "include::part.adoc[]\n"
        }
    })));

    assert_eq!(jobs.len(), 2);
    let project = jobs
        .iter()
        .find(|job| job.uri == "file:///book/root.adoc")
        .and_then(|job| job.prepared_request.as_ref())
        .expect("root project request");
    assert_eq!(project.request.targets.len(), 1);
    assert_eq!(project.request.sources.len(), 2);
    assert!(
        project
            .request
            .sources
            .iter()
            .all(|source| !source.source_id.as_str().starts_with("file:"))
    );
}

#[test]
fn session_tracks_multiple_project_roots_with_one_revision() {
    let mut session = Session::default();
    session.initialize(&initialize_params(&[
        "file:///workspace/first/",
        "file:///workspace/second/",
    ]));

    assert_eq!(
        session.workspace_roots(),
        vec![
            uri("file:///workspace/first/"),
            uri("file:///workspace/second/")
        ]
    );
    let initialized_revision = session.input_revision();
    let initialized_epoch = session.workspace_input_epoch();

    let _jobs = session.workspace_folders_changed(typed(json!({
        "event": {
            "removed": [{
                "uri": "file:///workspace/first/",
                "name": "first"
            }],
            "added": [{
                "uri": "file:///workspace/third/",
                "name": "third"
            }]
        }
    })));
    assert_eq!(session.input_revision(), initialized_revision + 1);
    assert_eq!(session.workspace_input_epoch(), initialized_epoch + 1);
    assert_eq!(
        session.workspace_roots(),
        vec![
            uri("file:///workspace/second/"),
            uri("file:///workspace/third/")
        ]
    );
}

#[test]
fn open_change_and_close_update_one_session_document() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let initialized_revision = session.input_revision();

    open(&mut session, "file:///guide.adoc", 3, "= Old\n");
    assert_eq!(session.input_revision(), initialized_revision + 1);
    let document = session
        .documents
        .get("file:///guide.adoc")
        .expect("open document");
    assert_eq!(document.document_input.revision.version, 3);
    assert_eq!(document.document_input.source.as_ref(), "= Old\n");
    let source_id = document.source_id.clone();
    let analysis_source_id = document
        .analysis()
        .and_then(adocweave::Analysis::source_id)
        .expect("project source ID");
    assert_eq!(
        document
            .view
            .as_ref()
            .and_then(|view| view.sources.get(analysis_source_id))
            .map(|source| source.uri.as_str()),
        Some("file:///guide.adoc")
    );
    assert!(session.documents.snapshot("file:///guide.adoc").is_some());

    let open_revision = session.input_revision();
    assert!(
        !change(
            &mut session,
            "file:///guide.adoc",
            3,
            json!([{"text": "= Ignored\n"}])
        )
        .expect("stale change is ignored")
    );
    assert_eq!(session.input_revision(), open_revision);

    assert!(
        change(
            &mut session,
            "file:///guide.adoc",
            4,
            json!([{"text": "= New\n"}])
        )
        .expect("new change")
    );
    assert_eq!(session.input_revision(), open_revision + 1);
    let document = session
        .documents
        .get("file:///guide.adoc")
        .expect("changed document");
    assert_eq!(document.document_input.revision.version, 4);
    assert_eq!(document.document_input.source.as_ref(), "= New\n");
    assert_eq!(document.source_id, source_id);
    let analysis_source_id = document
        .analysis()
        .and_then(adocweave::Analysis::source_id)
        .expect("project source ID");
    assert_eq!(
        document
            .view
            .as_ref()
            .and_then(|view| view.sources.get(analysis_source_id))
            .map(|source| source.uri.as_str()),
        Some(source_id.as_str())
    );

    let cancellation = session
        .document_cancellation(&uri("file:///guide.adoc"))
        .expect("document cancellation");
    let outcome = session.close(&uri("file:///guide.adoc"));
    assert!(outcome.closed);
    assert!(outcome.reanalysis_jobs.is_empty());
    assert!(cancellation.is_cancelled());
    assert_eq!(session.input_revision(), open_revision + 2);
    assert!(session.documents.get("file:///guide.adoc").is_none());
}

#[test]
fn effective_server_setting_changes_advance_session_revision() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let initialized_revision = session.input_revision();

    let jobs = session
        .update_configuration(json!({
            "adocweave": {"debounceMs": 30, "enabledRules": []}
        }))
        .expect("unchanged settings");
    assert!(jobs.is_empty());
    assert_eq!(session.input_revision(), initialized_revision);

    let jobs = session
        .update_configuration(json!({
            "adocweave": {"debounceMs": 75, "enabledRules": []}
        }))
        .expect("execution-only setting");
    assert!(jobs.is_empty());
    assert_eq!(session.input_revision(), initialized_revision);
    assert_eq!(session.debounce_ms(), 75);

    let jobs = session
        .update_configuration(json!({
            "adocweave": {"debounceMs": 75, "enabledRules": ["macro-boundary"]}
        }))
        .expect("analysis setting");
    assert!(jobs.is_empty());
    assert_eq!(session.input_revision(), initialized_revision + 1);
}

#[test]
#[should_panic(expected = "Language Server session input revision exhausted")]
fn session_input_revision_is_never_reused_after_exhaustion() {
    let mut session = Session::default();
    session.set_input_revision_for_test(u64::MAX);
    let _ = session.begin_open(typed(json!({
        "textDocument": {
            "uri": "file:///guide.adoc",
            "languageId": "asciidoc",
            "version": 1,
            "text": "= Guide\n"
        }
    })));
}
#[test]
#[should_panic(expected = "Language Server workspace input epoch exhausted")]
fn workspace_input_epoch_is_never_reused_after_exhaustion() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    session.set_workspace_input_epoch_for_test(u64::MAX);
    let _ = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///.adocweave.toml", "type": 2}]
    })));
}
