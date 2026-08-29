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

    let changed = session.workspace_folders_changed(typed(json!({
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
    assert!(changed);
    assert_eq!(session.input_revision(), initialized_revision + 1);
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
    assert_eq!(document.request.revision.version, 3);
    assert_eq!(document.request.source.as_ref(), "= Old\n");
    let source_id = document.source_id.clone();
    assert_eq!(
        document.analysis().and_then(adocweave::Analysis::source_id),
        Some(&source_id)
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
    assert_eq!(document.request.revision.version, 4);
    assert_eq!(document.request.source.as_ref(), "= New\n");
    assert_eq!(document.source_id, source_id);
    assert_eq!(
        document.analysis().and_then(adocweave::Analysis::source_id),
        Some(&source_id)
    );

    let cancellation = session
        .document_cancellation(&uri("file:///guide.adoc"))
        .expect("document cancellation");
    let (closed, jobs) = session.close(&uri("file:///guide.adoc"));
    assert!(closed);
    assert!(jobs.is_empty());
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
        .expect("changed settings");
    assert!(jobs.is_empty());
    assert_eq!(session.input_revision(), initialized_revision + 1);
    assert_eq!(session.debounce_ms(), 75);
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
