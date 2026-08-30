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
    session.initialize(&initialize_params(&[]));

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

    assert_eq!(jobs.len(), 1);
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
fn changed_overlay_retries_a_completed_project_before_adoption() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    open(&mut session, "file:///book/part.adoc", 1, "old overlay\n");
    let root = session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": "file:///book/root.adoc",
                "languageId": "asciidoc",
                "version": 1,
                "text": "= Root\n"
            }
        })))
        .pop()
        .expect("root analysis");
    let completed = process_project_snapshot(root);
    let crate::service::ProjectAnalysisAction::Validate(completed) =
        session.project_processing_completed(completed)
    else {
        panic!("completed project must wait for observation validation");
    };

    let overlay_jobs = session
        .begin_change(typed(json!({
            "textDocument": {"uri": "file:///book/part.adoc", "version": 2},
            "contentChanges": [{"text": "new overlay\n"}]
        })))
        .expect("overlay change");
    assert_eq!(overlay_jobs.len(), 1);

    let crate::service::ProjectAnalysisAction::Retry(retry) =
        session.complete_analysis(validate(*completed))
    else {
        panic!("changed overlay must retry the completed root analysis");
    };
    let request = &retry
        .prepared_request
        .as_ref()
        .expect("retry request")
        .request;
    assert!(
        request
            .sources
            .iter()
            .any(|source| source.source.as_ref() == "new overlay\n")
    );
    assert!(
        session
            .documents
            .snapshot("file:///book/root.adoc")
            .is_none()
    );
}

#[test]
fn closed_overlay_retries_a_completed_project_before_adoption() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let overlay_uri = uri("file:///book/part.adoc");
    open(&mut session, overlay_uri.as_str(), 1, "overlay\n");
    let root = session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": "file:///book/root.adoc",
                "languageId": "asciidoc",
                "version": 1,
                "text": "= Root\n"
            }
        })))
        .pop()
        .expect("root analysis");
    let completed = process_project_snapshot(root);
    let crate::service::ProjectAnalysisAction::Validate(completed) =
        session.project_processing_completed(completed)
    else {
        panic!("completed project must wait for observation validation");
    };

    assert!(session.close(&overlay_uri).closed);
    let crate::service::ProjectAnalysisAction::Retry(retry) =
        session.complete_analysis(validate(*completed))
    else {
        panic!("closed overlay must retry the completed root analysis");
    };
    assert_eq!(
        retry
            .prepared_request
            .as_ref()
            .expect("retry request")
            .request
            .sources
            .len(),
        1
    );
    assert!(
        session
            .documents
            .snapshot("file:///book/root.adoc")
            .is_none()
    );
}

#[test]
fn project_error_uses_the_same_validation_and_adoption_path_as_success() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let job = session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": "file:///guide.adoc",
                "languageId": "asciidoc",
                "version": 1,
                "text": "= Guide\n"
            }
        })))
        .pop()
        .expect("analysis snapshot");
    let mut completion = process_project_snapshot(job);
    completion.outcome = crate::service::ProjectAnalysisOutcome::Processed(Err(
        adocweave_project::ProjectError::TargetSelection(
            adocweave_project::TargetSelectionError::InvalidGlob {
                pattern: "[".to_owned(),
            },
        ),
    ));

    let crate::service::ProjectAnalysisAction::Validate(completion) =
        session.project_processing_completed(completion)
    else {
        panic!("project error must be validated");
    };
    let next = session.complete_analysis(validate(*completion));
    assert!(matches!(
        next,
        crate::service::ProjectAnalysisAction::Publish { .. }
    ));
    assert_eq!(
        session
            .documents
            .get("file:///guide.adoc")
            .and_then(|document| document.project_problem.as_ref())
            .map(|problem| problem.code.as_str()),
        Some("project-target-error")
    );
}

#[test]
fn closing_document_discards_project_worker_completion() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let document_uri = uri("file:///guide.adoc");
    let job = session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri.clone(),
                "languageId": "asciidoc",
                "version": 1,
                "text": "= Guide\n"
            }
        })))
        .pop()
        .expect("analysis snapshot");
    let completion = process_project_snapshot(job);

    assert!(session.close(&document_uri).closed);
    assert!(matches!(
        session.project_processing_completed(completion),
        crate::service::ProjectAnalysisAction::Ignore
    ));
}

#[test]
fn session_tracks_multiple_workspace_roots() {
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

    open(&mut session, "file:///guide.adoc", 3, "= Old\n");
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

    let open_generation = document.document_input.revision.generation;
    assert!(
        !change(
            &mut session,
            "file:///guide.adoc",
            3,
            json!([{"text": "= Ignored\n"}])
        )
        .expect("stale change is ignored")
    );
    assert_eq!(
        session
            .documents
            .get("file:///guide.adoc")
            .expect("open document")
            .document_input
            .revision
            .generation,
        open_generation
    );

    assert!(
        change(
            &mut session,
            "file:///guide.adoc",
            4,
            json!([{"text": "= New\n"}])
        )
        .expect("new change")
    );
    let document = session
        .documents
        .get("file:///guide.adoc")
        .expect("changed document");
    assert_eq!(document.document_input.revision.version, 4);
    assert!(document.document_input.revision.generation > open_generation);
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
    assert!(session.documents.get("file:///guide.adoc").is_none());
}

#[test]
fn effective_server_setting_changes_reanalyze_open_documents() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);

    let jobs = session
        .update_configuration(json!({
            "adocweave": {"debounceMs": 30, "enabledRules": []}
        }))
        .expect("unchanged settings");
    assert!(jobs.is_empty());

    let jobs = session
        .update_configuration(json!({
            "adocweave": {"debounceMs": 75, "enabledRules": []}
        }))
        .expect("execution-only setting");
    assert!(jobs.is_empty());
    assert_eq!(session.debounce_ms(), 75);

    let jobs = session
        .update_configuration(json!({
            "adocweave": {"debounceMs": 75, "enabledRules": ["macro-boundary"]}
        }))
        .expect("analysis setting");
    assert!(jobs.is_empty());
}
