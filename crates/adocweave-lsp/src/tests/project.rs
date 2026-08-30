//! Project observation and workspace-authority integration tests.

use std::fs;

use super::*;

fn initialize_root(session: &mut Session, root: &std::path::Path) {
    let root = lsp::Url::from_directory_path(root).expect("workspace URI");
    initialize_with_params(
        session,
        typed(json!({
            "processId": null,
            "rootUri": root,
            "capabilities": {}
        })),
    );
}

fn workspace_folder(name: &str, path: &std::path::Path) -> serde_json::Value {
    json!({
        "name": name,
        "uri": lsp::Url::from_file_path(path).expect("workspace URI")
    })
}

#[test]
fn unrelated_watched_file_does_not_reanalyze_an_open_document() {
    let root = tempfile::tempdir().expect("workspace");
    let document = root.path().join("guide.adoc");
    fs::write(&document, "= Guide\n").expect("document");
    let document_uri = lsp::Url::from_file_path(&document).expect("document URI");
    let unrelated_uri =
        lsp::Url::from_file_path(root.path().join("unrelated.txt")).expect("unrelated URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(&mut session, document_uri.as_str(), 1, "= Guide\n");

    let jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": unrelated_uri, "type": 1}]
    })));

    assert!(jobs.is_empty());
}

#[test]
fn include_change_reanalyzes_only_the_observing_document() {
    let root = tempfile::tempdir().expect("workspace");
    let include = root.path().join("part.txt");
    let first = root.path().join("first.adoc");
    let second = root.path().join("second.adoc");
    fs::write(&include, "old\n").expect("include");
    fs::write(&first, "include::part.txt[]\n").expect("first document");
    fs::write(&second, "= Second\n").expect("second document");
    let include_uri = lsp::Url::from_file_path(&include).expect("include URI");
    let first_uri = lsp::Url::from_file_path(&first).expect("first URI");
    let second_uri = lsp::Url::from_file_path(&second).expect("second URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(&mut session, first_uri.as_str(), 1, "include::part.txt[]\n");
    open(&mut session, second_uri.as_str(), 1, "= Second\n");
    fs::write(&include, "new\n").expect("changed include");

    let jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": include_uri, "type": 2}]
    })));

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].uri, first_uri.as_str());
}

#[test]
fn missing_include_creation_reanalyzes_the_document() {
    let root = tempfile::tempdir().expect("workspace");
    let document = root.path().join("guide.adoc");
    let include = root.path().join("missing.txt");
    fs::write(&document, "include::missing.txt[]\n").expect("document");
    let document_uri = lsp::Url::from_file_path(&document).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include).expect("include URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(
        &mut session,
        document_uri.as_str(),
        1,
        "include::missing.txt[]\n",
    );
    fs::write(&include, "created\n").expect("created include");

    let jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": include_uri, "type": 1}]
    })));

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].uri, document_uri.as_str());
}

#[test]
fn project_config_creation_reanalyzes_the_document() {
    let root = tempfile::tempdir().expect("workspace");
    let document = root.path().join("guide.adoc");
    let config = root.path().join(".adocweave.toml");
    fs::write(&document, "= Guide\n").expect("document");
    let document_uri = lsp::Url::from_file_path(&document).expect("document URI");
    let config_uri = lsp::Url::from_file_path(&config).expect("config URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(&mut session, document_uri.as_str(), 1, "= Guide\n");
    fs::write(&config, "schema-version = 2\n").expect("config");

    let jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": config_uri, "type": 1}]
    })));

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].uri, document_uri.as_str());
}

#[test]
fn innermost_folder_and_file_folder_select_the_expected_authority() {
    let root = tempfile::tempdir().expect("workspace");
    let nested = root.path().join("nested");
    let sibling = root.path().join("sibling");
    fs::create_dir(&nested).expect("nested folder");
    fs::create_dir(&sibling).expect("sibling folder");
    let document = nested.join("guide.adoc");
    fs::write(root.path().join("outer.txt"), "outer\n").expect("outer resource");
    fs::write(sibling.join("secret.txt"), "secret\n").expect("sibling resource");
    let source = "include::../outer.txt[]\ninclude::../sibling/secret.txt[]\n";
    fs::write(&document, source).expect("document");
    let document_uri = lsp::Url::from_file_path(&document).expect("document URI");

    let mut nested_session = Session::default();
    initialize_with_params(
        &mut nested_session,
        typed(json!({
            "processId": null,
            "capabilities": {"workspace": {"workspaceFolders": true}},
            "workspaceFolders": [
                workspace_folder("root", root.path()),
                workspace_folder("nested", &nested),
                workspace_folder("sibling", &sibling)
            ]
        })),
    );
    let nested_job = nested_session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": source
            }
        })))
        .pop()
        .expect("nested analysis");
    assert_eq!(
        nested_job
            .prepared_request
            .as_ref()
            .expect("prepared request")
            .request
            .authority
            .project_root(),
        nested
    );
    let completed = process_project_snapshot(nested_job);
    let crate::service::ProjectAnalysisOutcome::Processed(Ok(result)) = completed.outcome else {
        panic!("project processing result");
    };
    let resources = result.targets[0]
        .resources
        .iter()
        .filter(|resource| resource.kind == adocweave_project::ProjectResourceKind::Include)
        .collect::<Vec<_>>();
    assert_eq!(resources.len(), 1);
    assert!(
        resources.iter().all(|resource| {
            matches!(
                resource.outcome,
                adocweave_project::ProjectResourceOutcome::Failed(
                    adocweave_project::ProjectResourceFailure::Rejected(ref error)
                ) if error.code == adocweave_project::ProjectResourceErrorCode::OutsideAuthority
            )
        }),
        "{resources:#?}"
    );

    let sibling_job = nested_session
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "include::../sibling/secret.txt[]\n"}]
        })))
        .expect("sibling request")
        .pop()
        .expect("sibling analysis");
    let sibling_completed = process_project_snapshot(sibling_job);
    let crate::service::ProjectAnalysisOutcome::Processed(Ok(sibling_result)) =
        sibling_completed.outcome
    else {
        panic!("sibling project processing result");
    };
    assert!(sibling_result.targets[0].resources.iter().any(|resource| {
        matches!(
            resource.outcome,
            adocweave_project::ProjectResourceOutcome::Failed(
                adocweave_project::ProjectResourceFailure::Rejected(ref error)
            ) if resource.kind == adocweave_project::ProjectResourceKind::Include
                && error.code == adocweave_project::ProjectResourceErrorCode::OutsideAuthority
        )
    }));

    let mut file_session = Session::default();
    initialize_with_params(
        &mut file_session,
        typed(json!({
            "processId": null,
            "capabilities": {"workspace": {"workspaceFolders": true}},
            "workspaceFolders": [workspace_folder("guide", &document)]
        })),
    );
    let file_job = file_session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": source
            }
        })))
        .pop()
        .expect("file analysis");
    assert_eq!(
        file_job
            .prepared_request
            .as_ref()
            .expect("prepared request")
            .request
            .authority
            .project_root(),
        nested
    );
}

#[test]
fn config_and_local_target_changes_reanalyze_only_the_observing_document() {
    let root = tempfile::tempdir().expect("workspace");
    let config = root.path().join(".adocweave.toml");
    let target = root.path().join("target.adoc");
    let first = root.path().join("first.adoc");
    let second = root.path().join("second.adoc");
    fs::write(
        &config,
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    )
    .expect("config");
    fs::write(&target, "= Target\n").expect("target");
    fs::write(&first, "xref:target.adoc[Target]\n").expect("first document");
    fs::write(&second, "= Second\n").expect("second document");
    let config_uri = lsp::Url::from_file_path(&config).expect("config URI");
    let target_uri = lsp::Url::from_file_path(&target).expect("target URI");
    let first_uri = lsp::Url::from_file_path(&first).expect("first URI");
    let second_uri = lsp::Url::from_file_path(&second).expect("second URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(
        &mut session,
        first_uri.as_str(),
        1,
        "xref:target.adoc[Target]\n",
    );
    open(&mut session, second_uri.as_str(), 1, "= Second\n");

    let target_jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": target_uri, "type": 2}]
    })));
    assert_eq!(target_jobs.len(), 1);
    assert_eq!(target_jobs[0].uri, first_uri.as_str());
    adopt(
        &mut session,
        target_jobs.into_iter().next().expect("target job"),
    );

    let config_jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": config_uri, "type": 2}]
    })));
    assert_eq!(config_jobs.len(), 2);
    assert_eq!(
        config_jobs
            .iter()
            .map(|job| job.uri.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([first_uri.as_str(), second_uri.as_str()])
    );
}

#[test]
fn closing_one_root_removes_only_its_observations() {
    let root = tempfile::tempdir().expect("workspace");
    let include = root.path().join("shared.txt");
    let first = root.path().join("first.adoc");
    let second = root.path().join("second.adoc");
    fs::write(&include, "shared\n").expect("include");
    let include_uri = lsp::Url::from_file_path(&include).expect("include URI");
    let first_uri = lsp::Url::from_file_path(&first).expect("first URI");
    let second_uri = lsp::Url::from_file_path(&second).expect("second URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(
        &mut session,
        first_uri.as_str(),
        1,
        "include::shared.txt[]\n",
    );
    open(
        &mut session,
        second_uri.as_str(),
        1,
        "include::shared.txt[]\n",
    );

    assert!(session.close(&first_uri).closed);
    let jobs = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": include_uri, "type": 2}]
    })));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].uri, second_uri.as_str());
}

#[test]
fn watch_overflow_reanalyzes_each_open_document_once() {
    let root = tempfile::tempdir().expect("workspace");
    let first_uri = lsp::Url::from_file_path(root.path().join("first.adoc")).expect("first URI");
    let second_uri = lsp::Url::from_file_path(root.path().join("second.adoc")).expect("second URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(&mut session, first_uri.as_str(), 1, "= First\n");
    open(&mut session, second_uri.as_str(), 1, "= Second\n");
    let changes = (0..=crate::service::MAX_WORKSPACE_WATCH_CHANGES)
        .map(|index| {
            json!({
                "uri": lsp::Url::from_file_path(root.path().join(format!("changed-{index}.txt")))
                    .expect("changed URI"),
                "type": 2
            })
        })
        .collect::<Vec<_>>();

    let jobs = session.handle_workspace_files_changed(typed(json!({"changes": changes})));

    assert_eq!(jobs.len(), 2);
    assert_eq!(
        jobs.iter()
            .map(|job| job.uri.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([first_uri.as_str(), second_uri.as_str()])
    );
}

#[test]
fn repeated_watch_events_count_toward_the_overflow_limit() {
    let root = tempfile::tempdir().expect("workspace");
    let document_uri =
        lsp::Url::from_file_path(root.path().join("guide.adoc")).expect("document URI");
    let repeated_uri =
        lsp::Url::from_file_path(root.path().join("unrelated.txt")).expect("repeated URI");
    let mut session = Session::default();
    initialize_root(&mut session, root.path());
    open(&mut session, document_uri.as_str(), 1, "= Guide\n");
    let changes = (0..=crate::service::MAX_WORKSPACE_WATCH_CHANGES)
        .map(|_| json!({"uri": repeated_uri, "type": 2}))
        .collect::<Vec<_>>();

    let jobs = session.handle_workspace_files_changed(typed(json!({"changes": changes})));

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].uri, document_uri.as_str());
}
