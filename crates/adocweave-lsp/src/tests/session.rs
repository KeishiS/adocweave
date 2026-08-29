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
    let initialized_epoch = session.workspace_input_epoch();

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
fn project_configuration_change_invalidates_jobs_before_rebuild_finishes() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let job = session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": "file:///guide.adoc",
                "languageId": "asciidoc",
                "version": 1,
                "text": "= Before\n"
            }
        })))
        .remove(0);
    let result = job
        .request
        .analyze(&adocweave::NeverCancel)
        .expect("analysis before configuration change");
    let previous_revision = session.input_revision();
    let previous_epoch = session.workspace_input_epoch();

    let outcome = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///.adocweave.toml", "type": 2}]
    })));

    assert!(outcome.cancel_recovery_timer);
    assert!(outcome.rebuild.is_some());
    assert!(outcome.jobs.is_empty());
    assert!(job.cancellation.is_cancelled());
    assert_eq!(session.input_revision(), previous_revision + 1);
    assert_eq!(session.workspace_input_epoch(), previous_epoch + 1);
    assert_eq!(
        session
            .documents
            .get("file:///guide.adoc")
            .expect("open document")
            .input_revision,
        session.input_revision()
    );
    assert_eq!(session.adopt(&job, result), Adoption::Stale);
}

#[test]
fn oversized_watch_batch_invalidates_jobs_before_recovery_scan() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    let job = session
        .begin_open(typed(json!({
            "textDocument": {
                "uri": "file:///guide.adoc",
                "languageId": "asciidoc",
                "version": 1,
                "text": "= Before\n"
            }
        })))
        .remove(0);
    let result = job
        .request
        .analyze(&adocweave::NeverCancel)
        .expect("analysis before watched changes");
    let previous_revision = session.input_revision();
    let previous_epoch = session.workspace_input_epoch();
    let changes = (0..=10_000)
        .map(|index| {
            lsp::FileEvent::new(
                uri(&format!("file:///watched/{index}.adoc")),
                lsp::FileChangeType::CHANGED,
            )
        })
        .collect();

    let outcome =
        session.handle_workspace_files_changed(lsp::DidChangeWatchedFilesParams { changes });

    assert!(outcome.rebuild.is_none());
    assert!(outcome.recovery_generation.is_some());
    assert!(outcome.jobs.is_empty());
    assert!(job.cancellation.is_cancelled());
    assert_eq!(session.input_revision(), previous_revision + 1);
    assert_eq!(session.workspace_input_epoch(), previous_epoch + 1);
    assert_eq!(session.adopt(&job, result), Adoption::Stale);
}

fn session_with_published_result() -> (Session, lsp::Url) {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    session
        .update_configuration(json!({"enabledRules": ["macro-boundary"]}))
        .expect("diagnostic configuration");
    let document_uri = uri("file:///published.adoc");
    open(
        &mut session,
        document_uri.as_str(),
        1,
        "= Published\n\n日xref:guide.adoc[Guide]\n",
    );
    assert!(session.documents.snapshot(document_uri.as_str()).is_some());
    assert!(
        !session
            .diagnostics(&document_uri)
            .expect("published diagnostics")
            .diagnostics
            .is_empty()
    );
    assert!(
        session
            .hover(&document_uri, lsp::Position::new(0, 3))
            .expect("published hover")
            .is_some()
    );
    (session, document_uri)
}

fn assert_published_result_is_cleared(session: &Session, document_uri: &lsp::Url) {
    assert!(session.documents.snapshot(document_uri.as_str()).is_none());
    assert!(
        session
            .diagnostics(document_uri)
            .expect("diagnostics while rebuilding")
            .diagnostics
            .is_empty()
    );
    assert!(
        session
            .hover(document_uri, lsp::Position::new(0, 3))
            .expect("hover while rebuilding")
            .is_none()
    );
}

#[test]
fn rebuilding_clears_published_results_for_configuration_root_and_batch_changes() {
    let (mut configuration, document_uri) = session_with_published_result();
    let _ = configuration.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///.adocweave.toml", "type": 2}]
    })));
    assert_published_result_is_cleared(&configuration, &document_uri);

    let (mut roots, document_uri) = session_with_published_result();
    assert!(roots.workspace_folders_changed(typed(json!({
        "event": {
            "removed": [],
            "added": [{"uri": "file:///new-root/", "name": "new-root"}]
        }
    }))));
    assert_published_result_is_cleared(&roots, &document_uri);

    let (mut batch, document_uri) = session_with_published_result();
    let changes = (0..=10_000)
        .map(|index| {
            lsp::FileEvent::new(
                uri(&format!("file:///watched/{index}.adoc")),
                lsp::FileChangeType::CHANGED,
            )
        })
        .collect();
    let _ = batch.handle_workspace_files_changed(lsp::DidChangeWatchedFilesParams { changes });
    assert_published_result_is_cleared(&batch, &document_uri);
}

#[test]
fn open_and_change_wait_for_rebuild_before_creating_analysis_jobs() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    open(&mut session, "file:///changed.adoc", 1, "= Before\n");
    let _ = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///.adocweave.toml", "type": 2}]
    })));

    let changed_jobs = session
        .begin_change(typed(json!({
            "textDocument": {"uri": "file:///changed.adoc", "version": 2},
            "contentChanges": [{"text": "= After\n"}]
        })))
        .expect("change while rebuilding");
    let opened_jobs = session.begin_open(typed(json!({
        "textDocument": {
            "uri": "file:///opened.adoc",
            "languageId": "asciidoc",
            "version": 1,
            "text": "= Opened\n"
        }
    })));
    assert!(changed_jobs.is_empty());
    assert!(opened_jobs.is_empty());
    assert_eq!(
        session
            .documents
            .get("file:///changed.adoc")
            .expect("changed document")
            .request
            .source
            .as_ref(),
        "= After\n"
    );
    assert!(session.documents.get("file:///opened.adoc").is_some());
    assert!(session.documents.snapshot("file:///changed.adoc").is_none());
    assert!(session.documents.snapshot("file:///opened.adoc").is_none());

    let scan = session.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = session.apply_workspace_scan(scan).jobs;
    assert_eq!(jobs.len(), 2);
    assert!(
        jobs.iter()
            .any(|job| job.request.source.as_ref() == "= After\n")
    );
    assert!(
        jobs.iter()
            .any(|job| job.request.source.as_ref() == "= Opened\n")
    );
}

#[test]
fn workspace_error_epoch_survives_document_changes_and_hides_superseded_errors() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    open(&mut session, "file:///guide.adoc", 1, "= Before\n");

    let _ = session.workspace_scan_failed("old workspace failure".to_owned());
    let epoch = session.workspace_input_epoch();
    assert!(
        session
            .diagnostics(&uri("file:///guide.adoc"))
            .expect("old failure diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("old workspace failure"))
    );

    assert!(
        change(
            &mut session,
            "file:///guide.adoc",
            2,
            json!([{"text": "= Changed\n"}])
        )
        .expect("document change")
    );
    assert_eq!(session.workspace_input_epoch(), epoch);
    assert!(
        session
            .diagnostics(&uri("file:///guide.adoc"))
            .expect("failure after document change")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("old workspace failure"))
    );

    let outcome = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///.adocweave.toml", "type": 2}]
    })));
    assert_eq!(session.workspace_input_epoch(), epoch + 1);
    assert!(
        session
            .diagnostics(&uri("file:///guide.adoc"))
            .expect("diagnostics during replacement")
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("old workspace failure"))
    );

    let (sequence, _) = outcome.rebuild.expect("replacement scan").into_parts();
    let transition = session
        .complete_workspace_scan(crate::workspace_scan::WorkspaceScanned::new(
            sequence,
            Err("current workspace failure".to_owned()),
        ))
        .expect("accepted failed scan");
    assert!(transition.jobs.is_empty());
    assert!(
        session
            .diagnostics(&uri("file:///guide.adoc"))
            .expect("current failure diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("current workspace failure"))
    );
}

#[test]
fn workspace_input_and_watch_errors_follow_the_workspace_epoch() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-session-errors-{unique}"));
    let docs = root.join("docs");
    let other = root.join("other");
    fs::create_dir_all(&docs).expect("docs");
    fs::create_dir_all(&other).expect("other");
    let config_path = root.join(adocweave_config::FILE_NAME);
    let document_path = docs.join("root.adoc");
    let include_path = docs.join("part.adoc");
    let rejected_path = other.join("rejected.adoc");
    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\"docs\"]\n",
    )
    .expect("configuration");
    fs::write(&document_path, "include::part.adoc[]\n").expect("document");
    fs::write(&include_path, "part\n").expect("include");
    fs::write(&rejected_path, "rejected\n").expect("rejected document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let rejected_uri = lsp::Url::from_file_path(&rejected_path).expect("rejected URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("configuration URI");
    let mut session = Session::default();
    initialize_with_params(
        &mut session,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    let scan = session.plan_workspace_scan(&adocweave::NeverCancel);
    let _ = session.apply_workspace_scan(scan);
    open(
        &mut session,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );

    fs::write(&include_path, [0xff]).expect("invalid include");
    for job in session
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 2}]
        })))
        .jobs
    {
        adopt(&mut session, job);
    }
    let rejected_jobs = session.begin_open(typed(json!({
        "textDocument": {
            "uri": rejected_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "rejected\n"
        }
    })));
    assert_eq!(rejected_jobs.len(), 1);
    for job in rejected_jobs {
        adopt(&mut session, job);
    }
    let current = session
        .diagnostics(&document_uri)
        .expect("current workspace errors");
    assert!(current.diagnostics.iter().all(|diagnostic| {
        !diagnostic
            .message
            .contains("outside configured resource roots")
    }));
    assert!(
        current
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("UTF-8"))
    );
    let rejected = session
        .diagnostics(&rejected_uri)
        .expect("rejected document diagnostics");
    assert!(rejected.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("outside configured resource roots")
    }));

    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\".\"]\n",
    )
    .expect("expanded configuration");
    let replacement = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": config_uri, "type": 2}]
    })));
    let rebuilding = session
        .diagnostics(&document_uri)
        .expect("rebuilding diagnostics");
    assert!(rebuilding.diagnostics.iter().all(|diagnostic| {
        !diagnostic
            .message
            .contains("outside configured resource roots")
            && !diagnostic.message.contains("UTF-8")
    }));
    assert!(
        session
            .diagnostics(&rejected_uri)
            .expect("rejected document while rebuilding")
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic
                .message
                .contains("outside configured resource roots"))
    );

    fs::write(&include_path, "restored\n").expect("restored include");
    let scan = session.plan_workspace_scan(&adocweave::NeverCancel);
    let (sequence, _) = replacement
        .rebuild
        .expect("configuration replacement")
        .into_parts();
    let completed = session
        .complete_workspace_scan(crate::workspace_scan::WorkspaceScanned::new(
            sequence,
            Ok(scan),
        ))
        .expect("successful replacement");
    assert_eq!(completed.jobs.len(), 2);
    let recovered = session
        .diagnostics(&document_uri)
        .expect("recovered diagnostics");
    assert!(recovered.diagnostics.iter().all(|diagnostic| {
        !diagnostic
            .message
            .contains("outside configured resource roots")
            && !diagnostic.message.contains("UTF-8")
    }));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn failed_rebuild_retries_after_watch_and_analyzes_latest_open_inputs_once() {
    let mut session = Session::default();
    initialize(&mut session, &["utf-16"]);
    open(&mut session, "file:///changed.adoc", 1, "= Before\n");
    let outcome = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///.adocweave.toml", "type": 2}]
    })));
    let (failed_sequence, _) = outcome.rebuild.expect("replacement scan").into_parts();
    let failed = session
        .complete_workspace_scan(crate::workspace_scan::WorkspaceScanned::new(
            failed_sequence,
            Err("workspace worker failed".to_owned()),
        ))
        .expect("failed scan completion");
    assert!(failed.jobs.is_empty());

    assert!(
        session
            .begin_change(typed(json!({
                "textDocument": {"uri": "file:///changed.adoc", "version": 2},
                "contentChanges": [{"text": "= Latest\n"}]
            })))
            .expect("change while waiting for retry")
            .is_empty()
    );
    assert!(
        session
            .begin_open(typed(json!({
                "textDocument": {
                    "uri": "file:///opened.adoc",
                    "languageId": "asciidoc",
                    "version": 1,
                    "text": "= Opened\n"
                }
            })))
            .is_empty()
    );

    let watched = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": "file:///ordinary.adoc", "type": 2}]
    })));
    let generation = watched
        .recovery_generation
        .expect("failed scan recovery generation");
    let retry = session
        .request_workspace_scan_recovery(crate::workspace_scan::WorkspaceScanRecovery::new(
            generation,
        ))
        .expect("retry scan");
    let (retry_sequence, _) = retry.into_parts();
    let scan = session.plan_workspace_scan(&adocweave::NeverCancel);
    let recovered = session
        .complete_workspace_scan(crate::workspace_scan::WorkspaceScanned::new(
            retry_sequence,
            Ok(scan),
        ))
        .expect("successful retry");

    assert_eq!(recovered.jobs.len(), 2);
    assert_eq!(
        recovered
            .jobs
            .iter()
            .filter(|job| job.uri == "file:///changed.adoc")
            .count(),
        1
    );
    assert_eq!(
        recovered
            .jobs
            .iter()
            .filter(|job| job.uri == "file:///opened.adoc")
            .count(),
        1
    );
    assert!(recovered.jobs.iter().any(|job| {
        job.uri == "file:///changed.adoc" && job.request.source.as_ref() == "= Latest\n"
    }));
    assert!(recovered.jobs.iter().any(|job| {
        job.uri == "file:///opened.adoc" && job.request.source.as_ref() == "= Opened\n"
    }));
    assert!(
        session
            .diagnostics(&uri("file:///changed.adoc"))
            .expect("diagnostics after recovery")
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("workspace worker failed"))
    );
}

#[test]
fn failed_structural_install_stays_pending_until_recovery_installs_latest_open_inputs() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-structural-retry-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let config_path = root.join(adocweave_config::FILE_NAME);
    let changed_path = root.join("changed.adoc");
    let closed_path = root.join("closed.adoc");
    let opened_path = root.join("opened.adoc");
    fs::write(&config_path, "schema-version = 2\n").expect("configuration");
    fs::write(&changed_path, "= Changed disk\n").expect("changed document");
    fs::write(&closed_path, "= Closed disk\n").expect("closed document");
    fs::write(&opened_path, "= Opened disk\n").expect("opened document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("configuration URI");
    let changed_uri = lsp::Url::from_file_path(&changed_path).expect("changed URI");
    let closed_uri = lsp::Url::from_file_path(&closed_path).expect("closed URI");
    let opened_uri = lsp::Url::from_file_path(&opened_path).expect("opened URI");
    let mut session = Session::default();
    initialize_with_params(
        &mut session,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut session, changed_uri.as_str(), 1, "= Changed open 1\n");
    open(&mut session, closed_uri.as_str(), 1, "= Closed overlay\n");

    fs::remove_file(&config_path).expect("remove configuration");
    fs::create_dir(&config_path).expect("unreadable configuration entry");
    let replacement = session.handle_workspace_files_changed(typed(json!({
        "changes": [{"uri": config_uri, "type": 2}]
    })));
    let (failed_sequence, _) = replacement
        .rebuild
        .expect("structural replacement")
        .into_parts();
    let failed_scan = session.plan_workspace_scan(&adocweave::NeverCancel);

    assert!(
        session
            .begin_change(typed(json!({
                "textDocument": {"uri": changed_uri, "version": 2},
                "contentChanges": [{"text": "= Changed open 2\n"}]
            })))
            .expect("change during rebuild")
            .is_empty()
    );
    assert!(
        session
            .begin_open(typed(json!({
                "textDocument": {
                    "uri": opened_uri,
                    "languageId": "asciidoc",
                    "version": 1,
                    "text": "= Opened 1\n"
                }
            })))
            .is_empty()
    );
    let (closed, close_jobs) = session.close(&closed_uri);
    assert!(closed);
    assert!(close_jobs.is_empty());
    let watched_before_failure = session.handle_workspace_files_changed(typed(json!({
        "changes": [{
            "uri": lsp::Url::from_file_path(root.join("before.txt")).expect("watch URI"),
            "type": 2
        }]
    })));
    assert!(watched_before_failure.jobs.is_empty());

    let failed = session
        .complete_workspace_scan(crate::workspace_scan::WorkspaceScanned::new(
            failed_sequence,
            Ok(failed_scan),
        ))
        .expect("failed structural install");
    assert!(failed.jobs.is_empty());
    assert!(session.workspace_rebuild_is_pending());
    let crate::workspace_scan::WorkspaceRecoveryTimerUpdate::Arm(_) = failed.recovery_timer else {
        panic!("failed structural install must arm recovery");
    };
    assert!(session.documents.snapshot(changed_uri.as_str()).is_none());
    assert!(session.documents.snapshot(opened_uri.as_str()).is_none());
    assert_eq!(
        session
            .workspace_resource(&changed_uri)
            .expect("preserved coherent overlay")
            .as_ref(),
        "= Changed open 1\n"
    );
    assert_eq!(
        session
            .workspace_resource(&closed_uri)
            .expect("not-yet-replaced closed overlay")
            .as_ref(),
        "= Closed overlay\n"
    );

    assert!(
        session
            .begin_change(typed(json!({
                "textDocument": {"uri": changed_uri, "version": 3},
                "contentChanges": [{"text": "= Changed latest\n"}]
            })))
            .expect("change after failed install")
            .is_empty()
    );
    assert!(
        session
            .begin_change(typed(json!({
                "textDocument": {"uri": opened_uri, "version": 2},
                "contentChanges": [{"text": "= Opened latest\n"}]
            })))
            .expect("opened change after failed install")
            .is_empty()
    );
    let watched_after_failure = session.handle_workspace_files_changed(typed(json!({
        "changes": [{
            "uri": lsp::Url::from_file_path(root.join("after.txt")).expect("watch URI"),
            "type": 2
        }]
    })));
    assert!(watched_after_failure.jobs.is_empty());
    let recovery_generation = watched_after_failure
        .recovery_generation
        .expect("recovery generation");

    fs::remove_dir(&config_path).expect("remove unreadable configuration entry");
    fs::write(&config_path, "schema-version = 2\n").expect("restored configuration");
    let retry = session
        .request_workspace_scan_recovery(crate::workspace_scan::WorkspaceScanRecovery::new(
            recovery_generation,
        ))
        .expect("recovery scan");
    let (retry_sequence, _) = retry.into_parts();
    let recovered_scan = session.plan_workspace_scan(&adocweave::NeverCancel);
    let recovered = session
        .complete_workspace_scan(crate::workspace_scan::WorkspaceScanned::new(
            retry_sequence,
            Ok(recovered_scan),
        ))
        .expect("successful recovery");

    assert!(!session.workspace_rebuild_is_pending());
    assert_eq!(recovered.jobs.len(), 2);
    assert_eq!(
        recovered
            .jobs
            .iter()
            .filter(|job| job.uri == changed_uri.as_str())
            .count(),
        1
    );
    assert_eq!(
        recovered
            .jobs
            .iter()
            .filter(|job| job.uri == opened_uri.as_str())
            .count(),
        1
    );
    assert!(recovered.jobs.iter().any(|job| {
        job.uri == changed_uri.as_str() && job.request.source.as_ref() == "= Changed latest\n"
    }));
    assert!(recovered.jobs.iter().any(|job| {
        job.uri == opened_uri.as_str() && job.request.source.as_ref() == "= Opened latest\n"
    }));
    assert!(
        recovered
            .jobs
            .iter()
            .all(|job| job.uri != closed_uri.as_str())
    );
    assert_eq!(
        session
            .workspace_resource(&closed_uri)
            .expect("closed disk resource")
            .as_ref(),
        "= Closed disk\n"
    );
    fs::remove_dir_all(root).expect("cleanup");
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
