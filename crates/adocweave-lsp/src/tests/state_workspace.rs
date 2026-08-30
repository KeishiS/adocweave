use super::*;
use crate::workspace::WorkspaceScanNotice;
use adocweave::CancellationCheck;

#[test]
fn scan_notice_episode_ends_after_a_complete_scan() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-scan-notice-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let config = root.join(adocweave_config::FILE_NAME);
    for name in ["a.adoc", "b.adoc", "c.adoc"] {
        fs::write(root.join(name), "text\n").expect("document");
    }
    fs::write(&config, "schema-version = 2\n[resources]\nmax-files = 1\n")
        .expect("limited configuration");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let expected = WorkspaceScanNotice::ProjectResourceLimit {
        project: config.clone(),
    };
    let mut service = Session::default();
    service.initialize(&typed(json!({
        "processId": null,
        "rootUri": root_uri,
        "capabilities": {}
    })));

    let first = service.plan_workspace_scan(&adocweave::NeverCancel);
    let first = service.apply_workspace_scan(first);
    assert!(first.installed);
    assert_eq!(first.notices, vec![expected.clone()]);

    let repeated = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert!(service.apply_workspace_scan(repeated).notices.is_empty());

    fs::write(&config, "schema-version = 99\n").expect("invalid configuration");
    let failed = service.plan_workspace_scan(&adocweave::NeverCancel);
    let failed = service.apply_workspace_scan(failed);
    assert!(!failed.installed);
    assert!(failed.notices.is_empty());
    fs::write(&config, "schema-version = 2\n[resources]\nmax-files = 1\n")
        .expect("recovered limited configuration");
    let recovered = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert!(service.apply_workspace_scan(recovered).notices.is_empty());

    fs::write(&config, "schema-version = 2\n[resources]\nmax-files = 8\n")
        .expect("complete configuration");
    let complete = service.plan_workspace_scan(&adocweave::NeverCancel);
    let complete = service.apply_workspace_scan(complete);
    assert!(complete.installed);
    assert!(complete.notices.is_empty());
    assert_eq!(service.workspace_analysis_count(), 3);

    fs::write(&config, "schema-version = 2\n[resources]\nmax-files = 1\n")
        .expect("limited configuration");
    let recurrence = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert_eq!(
        service.apply_workspace_scan(recurrence).notices,
        vec![expected]
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn workspace_folder_change_starts_a_new_scan_notice_episode() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-scan-notice-roots-{unique}"));
    let limited = base.join("limited");
    let added = base.join("added");
    fs::create_dir_all(&limited).expect("limited workspace");
    fs::create_dir_all(&added).expect("added workspace");
    let config = limited.join(adocweave_config::FILE_NAME);
    fs::write(&config, "schema-version = 2\n[resources]\nmax-files = 1\n")
        .expect("limited configuration");
    for name in ["a.adoc", "b.adoc"] {
        fs::write(limited.join(name), "text\n").expect("document");
    }
    let limited_uri = lsp::Url::from_directory_path(&limited).expect("limited URI");
    let added_uri = lsp::Url::from_directory_path(&added).expect("added URI");
    let expected = WorkspaceScanNotice::ProjectResourceLimit { project: config };
    let mut service = Session::default();
    service.initialize(&typed(json!({
        "processId": null,
        "workspaceFolders": [{"uri": limited_uri, "name": "limited"}],
        "capabilities": {"workspace": {"workspaceFolders": true}}
    })));

    let first = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert_eq!(
        service.apply_workspace_scan(first).notices,
        vec![expected.clone()]
    );
    let _jobs = service.workspace_folders_changed(typed(json!({
        "event": {
            "removed": [],
            "added": [{"uri": added_uri, "name": "added"}]
        }
    })));
    let changed = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert_eq!(
        service.apply_workspace_scan(changed).notices,
        vec![expected]
    );
    let repeated = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert!(service.apply_workspace_scan(repeated).notices.is_empty());

    fs::remove_dir_all(base).expect("cleanup");
}
#[test]
fn non_adoc_include_is_loaded_and_watched_without_becoming_an_analysis_root() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-text-include-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\".\"]\n",
    )
    .expect("configuration");
    let document_path = root.join("root.adoc");
    let include_path = root.join("part.txt");
    let source = "include::part.txt[]\n";
    fs::write(&document_path, source).expect("document");
    fs::write(&include_path, "first marker\n").expect("include");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    open(&mut service, document_uri.as_str(), 1, source);
    let analysis = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("workspace analysis");
    assert!(analysis.analysis.source().contains("first marker"));
    assert_eq!(service.workspace_analysis_count(), 1);
    let previous_revision = service.input_revision();
    let previous_cancellation = service
        .document_cancellation(&document_uri)
        .expect("document cancellation");

    fs::write(&include_path, "second marker\n").expect("changed include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 2}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1);
    assert!(previous_cancellation.is_cancelled());
    assert_eq!(service.input_revision(), previous_revision + 1);
    assert_eq!(jobs[0].input_revision, service.input_revision());
    for job in jobs {
        adopt(&mut service, job);
    }
    let analysis = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("updated workspace analysis");
    assert!(analysis.analysis.source().contains("second marker"));

    fs::remove_file(&include_path).expect("remove include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 3}]
        })))
        .jobs;
    for job in jobs {
        adopt(&mut service, job);
    }
    fs::write(&include_path, [0xff]).expect("invalid recreated include");
    let failed = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": include_uri, "type": 1}]
    })));
    assert_eq!(
        failed.journal.len(),
        1,
        "a known non-adoc include failure must survive an in-flight scan"
    );
    fs::write(&include_path, "restored marker\n").expect("restore include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 1}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1, "a known deleted include must remain watched");
    for job in jobs {
        adopt(&mut service, job);
    }
    let analysis = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("restored workspace analysis");
    assert!(analysis.analysis.source().contains("restored marker"));
    assert_eq!(service.workspace_analysis_count(), 2);

    assert!(
        change(
            &mut service,
            document_uri.as_str(),
            2,
            json!([{"text": "without include\n"}]),
        )
        .expect("remove include reference")
    );
    assert_eq!(
        service.workspace_analysis_count(),
        1,
        "an unreferenced include must release its retained resource"
    );
    fs::write(&include_path, "ignored marker\n").expect("change unreferenced include");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": include_uri, "type": 2}]
            })))
            .jobs
            .is_empty(),
        "an unreferenced non-adoc include must no longer be watched"
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn initially_missing_non_adoc_include_recovers_when_created() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-created-text-include-{unique}"));
    let generated = root.join("generated");
    fs::create_dir_all(&generated).expect("workspace");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        concat!(
            "schema-version = 2\n",
            "[resources]\ninclude = true\nroots = [\".\"]\n",
        ),
    )
    .expect("configuration");
    let document_path = root.join("root.adoc");
    let include_path = generated.join("part.txt");
    let source = "include::generated/part.txt[]\n";
    fs::write(&document_path, source).expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    open(&mut service, document_uri.as_str(), 1, source);
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("missing diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String("missing-resource".to_owned())))
    );

    fs::write(&include_path, "created marker\n").expect("create include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 1}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        adopt(&mut service, job);
    }
    let analysis = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("recovered workspace analysis");
    assert!(analysis.analysis.source().contains("created marker"));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn watched_disk_change_is_retained_below_an_open_overlay() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-open-disk-watch-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    fs::write(&document_path, "disk before\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, "open overlay\n");

    fs::write(&document_path, "disk after\n").expect("change disk source");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": document_uri, "type": 2}]
            })))
            .jobs
            .is_empty(),
        "the unchanged overlay must not be reanalyzed"
    );
    let outcome = service.close(&document_uri);
    assert!(outcome.reanalysis_jobs.is_empty());
    assert_eq!(
        service
            .workspace_resource(&document_uri)
            .expect("retained disk source")
            .as_ref(),
        "disk after\n"
    );

    open(&mut service, document_uri.as_str(), 1, "second overlay\n");
    fs::remove_file(&document_path).expect("delete disk source");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": document_uri, "type": 3}]
            })))
            .jobs
            .is_empty()
    );
    let _ = service.close(&document_uri);
    assert!(service.workspace_resource(&document_uri).is_none());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn pending_non_adoc_include_recovers_from_a_workspace_problem() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-pending-text-include-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\".\"]\n",
    )
    .expect("configuration");
    let document_path = root.join("root.adoc");
    let include_path = root.join("part.txt");
    let source = "include::part.txt[]\n";
    fs::write(&document_path, source).expect("document");
    fs::write(&include_path, "include::root.adoc[]\n").expect("cyclic include");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    open(&mut service, document_uri.as_str(), 1, source);
    assert!(
        service
            .documents
            .get(document_uri.as_str())
            .and_then(|document| document.expanded_analysis())
            .is_none(),
        "the circular include must remain a recoverable workspace problem"
    );

    fs::write(&include_path, "recovered marker\n").expect("repair include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 2}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1, "the pending include must reanalyze its root");
    for job in jobs {
        adopt(&mut service, job);
    }
    let analysis = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("recovered workspace analysis");
    assert!(analysis.analysis.source().contains("recovered marker"));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn oversized_watched_file_batch_requests_a_quiet_full_scan() {
    let mut service = Session::default();
    let changes = (0..=10_000)
        .map(|index| {
            json!({
                "uri": format!("file:///tmp/adocweave-watch-limit/{index}.adoc"),
                "type": 2
            })
        })
        .collect::<Vec<_>>();
    let outcome = service.workspace_files_changed_with_journal(typed(json!({
        "changes": changes
    })));

    assert!(outcome.recovery_required);
    assert!(outcome.jobs.is_empty());
    assert!(outcome.journal.is_empty());
}

#[test]
fn oversized_watched_file_error_requests_a_quiet_full_scan() {
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": "file:///tmp",
            "capabilities": {}
        })),
    );
    let uri = format!("file:///tmp/{}{}.adoc", "a".repeat(70_000), "missing");
    let outcome = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": uri, "type": 2}]
    })));

    assert!(outcome.recovery_required);
    assert!(outcome.journal.len() <= 1);
}

#[test]
fn failed_quiet_recovery_is_rearmed_by_the_next_watch_notification() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-watch-recovery-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let invalid_path = root.join("invalid.adoc");
    fs::write(&invalid_path, [0xff]).expect("invalid document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let invalid_uri = lsp::Url::from_file_path(&invalid_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri.clone(),
            "capabilities": {}
        })),
    );
    let oversized_uri = format!(
        "{}/{}.adoc",
        root_uri.as_str().trim_end_matches('/'),
        "a".repeat(70_000)
    );
    let overflow = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": oversized_uri, "type": 2}]
    })));
    assert!(overflow.recovery_required);

    let failed_scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let _ = service.apply_workspace_scan(failed_scan);
    fs::write(&invalid_path, "recovered\n").expect("repair document");
    let repaired = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": invalid_uri, "type": 2}]
    })));
    assert!(
        repaired.recovery_required,
        "a failed recovery must wait for and rearm on the next notification"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn failed_initial_scan_is_rearmed_by_the_next_watch_notification() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-initial-scan-recovery-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let invalid_path = root.join("invalid.adoc");
    fs::write(&invalid_path, [0xff]).expect("invalid document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let invalid_uri = lsp::Url::from_file_path(&invalid_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    fs::write(&invalid_path, "recovered\n").expect("repair document");
    let repaired = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": invalid_uri, "type": 2}]
    })));
    assert!(
        repaired.recovery_required,
        "a failed initial scan must rearm after the repaired file changes"
    );

    let recovered_scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let _ = service.apply_workspace_scan(recovered_scan);
    let settled = service.workspace_files_changed_with_journal(typed(json!({
        "changes": []
    })));
    assert!(
        !settled.recovery_required,
        "a successful full scan must clear the recovery request"
    );
    fs::remove_dir_all(root).expect("cleanup");
}
#[test]
fn stale_analysis_cannot_load_a_new_include_resource() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-stale-include-{unique}"));
    let excluded = root.join("generated");
    fs::create_dir_all(&excluded).expect("workspace directory");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        concat!(
            "schema-version = 2\n",
            "[resources]\ninclude = true\nroots = [\".\"]\n",
        ),
    )
    .expect("configuration");
    let document_path = root.join("root.adoc");
    let target_path = excluded.join("part.inc");
    fs::write(&document_path, "= Root\n").expect("root document");
    fs::write(&target_path, "stale included marker\n").expect("included document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, "= Root\n");

    let stale_job = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "include::generated/part.inc[]\n"}]
        })))
        .expect("first change")
        .pop()
        .expect("stale analysis");
    // The worker finishes against the state it started from, including reading
    // the include, but a newer revision arrives before the result comes back.
    let mut stale_job = stale_job;
    let project = stale_job.prepared_request.take().expect("project request");
    let analyzed = adocweave_project::process(project.request, stale_job.cancellation.as_ref())
        .expect("stale analysis completes");
    service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 3},
            "contentChanges": [{"text": "= Newer root\n"}]
        })))
        .expect("newer change");

    assert!(
        service
            .adopt_project_result(&stale_job, analyzed, project.source_index)
            .is_empty(),
        "a stale worker result must not publish anything"
    );
    let target = lsp::Url::from_file_path(&target_path).expect("target URI");
    assert!(
        service.workspace_copy().get(&target).is_none(),
        "a stale worker result must not leave the include it read"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn different_project_scopes_sharing_an_include_converge_on_the_current_generation() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-concurrent-include-{unique}"));
    let first_project = root.clone();
    let second_project = root.join("nested");
    let shared = second_project.join("shared");
    fs::create_dir_all(&second_project).expect("second project");
    fs::create_dir_all(&shared).expect("shared directory");
    for (project, max_files) in [(&first_project, 64), (&second_project, 65)] {
        fs::write(
            project.join(adocweave_config::FILE_NAME),
            format!(
                "schema-version = 2\n[resources]\ninclude = true\nroots = [\".\"]\nmax-files = {max_files}\n"
            ),
        )
        .expect("project configuration");
    }
    let first_path = first_project.join("root.adoc");
    let second_path = second_project.join("root.adoc");
    let first_source = "include::nested/shared/part.adoc[]\n";
    let second_source = "include::shared/part.adoc[]\n";
    fs::write(&first_path, first_source).expect("first root");
    fs::write(&second_path, second_source).expect("second root");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let first_uri = lsp::Url::from_file_path(&first_path).expect("first URI");
    let second_uri = lsp::Url::from_file_path(&second_path).expect("second URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    let first_job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": first_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": first_source
            }
        })))
        .pop()
        .expect("first job");
    let second_job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": second_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": second_source
            }
        })))
        .pop()
        .expect("second job");
    // Both immutable inputs were captured while the shared target was absent.
    // The workers must therefore acquire the same target-scope filesystem
    // session rather than reusing a resource from either snapshot.
    fs::write(shared.join("part.adoc"), "shared marker\n").expect("shared include");

    let first_config = first_job
        .project_context
        .as_ref()
        .expect("first workspace input")
        .config_sha256;
    let second_config = second_job
        .project_context
        .as_ref()
        .expect("second workspace input")
        .config_sha256;
    assert_ne!(first_config, second_config);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    for mut job in [first_job, second_job] {
        let sender = sender.clone();
        tokio::spawn(async move {
            let completed = tokio::task::spawn_blocking(move || {
                let project = job.prepared_request.take().expect("project request");
                let result = adocweave_project::process(project.request, job.cancellation.as_ref());
                (job, project.source_index, result)
            })
            .await
            .expect("analysis worker");
            sender.send(completed).await.expect("completion receiver");
        });
    }
    drop(sender);

    let mut refreshed = 0;
    while let Some((job, sources, result)) = receiver.recv().await {
        if let Some(mut retry) = service.refresh_stale_project(&job) {
            refreshed += 1;
            let project = retry
                .prepared_request
                .take()
                .expect("retry project request");
            let result = adocweave_project::process(project.request, retry.cancellation.as_ref())
                .expect("retry project analysis");
            let _ = service.adopt_project_result(&retry, result, project.source_index);
        } else {
            let result = result.expect("project analysis");
            let _ = service.adopt_project_result(&job, result, sources);
        }
    }
    assert!(refreshed > 0, "a stale project context must be retried");
    assert!(
        service
            .documents
            .get(first_uri.as_str())
            .expect("first document")
            .expanded_analysis()
            .expect("first analysis")
            .analysis
            .source()
            .contains("shared marker")
    );
    assert_eq!(
        service
            .documents
            .get(first_uri.as_str())
            .expect("first document")
            .document_input
            .revision
            .version,
        1
    );
    assert_eq!(
        service
            .documents
            .get(second_uri.as_str())
            .expect("second document")
            .document_input
            .revision
            .version,
        1
    );
    assert!(
        service
            .documents
            .get(second_uri.as_str())
            .expect("second document")
            .expanded_analysis()
            .expect("second analysis")
            .analysis
            .source()
            .contains("shared marker")
    );
    fs::remove_dir_all(root).expect("cleanup");
}
#[test]
fn standalone_open_documents_use_separate_parent_authorities_and_release_them_on_close() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-standalone-open-{unique}"));
    let first_root = base.join("first");
    let second_root = base.join("second");
    fs::create_dir_all(&first_root).expect("first document directory");
    fs::create_dir_all(&second_root).expect("second document directory");
    let first_path = first_root.join("guide.adoc");
    let second_path = second_root.join("guide.adoc");
    for root in [&first_root, &second_root] {
        fs::write(
            root.join(adocweave_config::FILE_NAME),
            "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
        )
        .expect("standalone project configuration");
        fs::write(root.join("asset.png"), [0x89, b'P', b'N', b'G']).expect("local target");
    }
    fs::write(first_root.join("part.adoc"), "first included marker\n").expect("first include");
    fs::write(second_root.join("part.adoc"), "second included marker\n").expect("second include");
    let first_uri = lsp::Url::from_file_path(&first_path).expect("first document URI");
    let second_uri = lsp::Url::from_file_path(&second_path).expect("second document URI");
    let mut service = Session::default();
    service.initialize(&typed(json!({
        "processId": null,
        "rootUri": null,
        "workspaceFolders": [],
        "capabilities": {"workspace": {"workspaceFolders": true}}
    })));

    let first_jobs = service.begin_open(typed(json!({
        "textDocument": {
            "uri": first_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "include::part.adoc[]\n\nimage::asset.png[]\n"
        }
    })));
    assert_eq!(first_jobs.len(), 1);
    assert_eq!(
        first_jobs[0]
            .project_context
            .as_ref()
            .expect("first project context")
            .authority_roots,
        vec![first_root.canonicalize().expect("first authority root")]
    );
    assert!(
        first_jobs[0]
            .project_context
            .as_ref()
            .expect("first project context")
            .project_config
            .local_targets
            .enabled
    );
    for job in first_jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .documents
            .get(first_uri.as_str())
            .and_then(|document| document.expanded_analysis())
            .expect("first expanded analysis")
            .analysis
            .source()
            .contains("first included marker")
    );
    assert!(
        service
            .diagnostics(&first_uri)
            .expect("first diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "local-target-missing".to_owned()
                )))
    );

    let second_jobs = service.begin_open(typed(json!({
        "textDocument": {
            "uri": second_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "include::part.adoc[]\n\nimage::asset.png[]\n"
        }
    })));
    assert_eq!(second_jobs.len(), 2);
    for job in &second_jobs {
        let expected = if job.uri == first_uri.as_str() {
            first_root.canonicalize().expect("first authority root")
        } else {
            second_root.canonicalize().expect("second authority root")
        };
        assert_eq!(
            job.project_context
                .as_ref()
                .expect("standalone project context")
                .authority_roots,
            vec![expected]
        );
    }
    for job in second_jobs {
        adopt(&mut service, job);
    }
    assert_eq!(service.workspace_analysis_count(), 2);

    let first_close = service.close(&first_uri);
    assert!(first_close.closed);
    assert_eq!(first_close.reanalysis_jobs.len(), 1);
    assert_eq!(service.workspace_analysis_count(), 1);
    let second_close = service.close(&second_uri);
    assert!(second_close.closed);
    assert!(second_close.reanalysis_jobs.is_empty());
    assert_eq!(service.workspace_analysis_count(), 0);

    fs::remove_dir_all(base).expect("cleanup");
}

#[test]
fn file_workspace_folder_analyzes_only_the_selected_document_as_a_root() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-single-file-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("document.adoc");
    let included_path = root.join("included.adoc");
    let unrelated_path = root.join("unrelated.adoc");
    let source = "include::included.adoc[]\n";
    fs::write(&document_path, source).expect("document");
    fs::write(&included_path, "included marker\n").expect("included document");
    fs::write(&unrelated_path, "unrelated marker\n").expect("unrelated document");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let included_uri = lsp::Url::from_file_path(&included_path).expect("included URI");
    let unrelated_uri = lsp::Url::from_file_path(&unrelated_path).expect("unrelated URI");

    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "workspaceFolders": [{"uri": document_uri, "name": "document.adoc"}],
            "capabilities": {"workspace": {"workspaceFolders": true}}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, source);

    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().all(|diagnostic| {
        diagnostic.code
            != Some(lsp::NumberOrString::String(
                "workspace-resource-error".to_owned(),
            ))
    }));
    let expanded = service
        .documents
        .get(document_uri.as_str())
        .expect("open document")
        .expanded_analysis()
        .expect("expanded analysis");
    assert!(expanded.analysis.source().contains("included marker"));
    assert!(!expanded.analysis.source().contains("unrelated marker"));
    assert_eq!(service.workspace_analysis_count(), 1);

    fs::write(&included_path, "changed disk marker\n").expect("changed included document");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": included_uri, "type": 2}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .documents
            .get(document_uri.as_str())
            .and_then(|document| document.expanded_analysis())
            .expect("reanalyzed selected document")
            .analysis
            .source()
            .contains("changed disk marker")
    );

    open(
        &mut service,
        included_uri.as_str(),
        1,
        "open overlay marker\n",
    );
    assert!(
        service
            .documents
            .get(document_uri.as_str())
            .and_then(|document| document.expanded_analysis())
            .expect("selected document with include overlay")
            .analysis
            .source()
            .contains("open overlay marker")
    );
    assert!(service.workspace_resource(&unrelated_uri).is_none());
    assert!(service.documents.get(unrelated_uri.as_str()).is_none());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn analysis_adoption_rejects_a_stale_workspace_generation() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root: PathBuf = std::env::temp_dir().join(format!("adocweave-stale-workspace-{unique}"));
    let root_path = root.join("root.adoc");
    let part_path = root.join("part.adoc");
    fs::create_dir_all(&root).expect("workspace");
    fs::write(&root_path, "include::part.adoc[]\n").expect("root document");
    fs::write(&part_path, "old\n").expect("part document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&root_path).expect("document URI");
    let part_uri = lsp::Url::from_file_path(&part_path).expect("part URI");

    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "rootUri": root_uri,
        "capabilities": {}
    }));
    initialize_with_params(&mut service, params);
    let job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": "include::part.adoc[]\n"
            }
        })))
        .into_iter()
        .next()
        .expect("analysis job");
    let analysis = analyze_document_input(&job, job.cancellation.as_ref());

    fs::write(&part_path, "new\n").expect("changed part");
    service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": part_uri, "type": 2}]
    })));

    assert_eq!(service.adopt(&job, analysis), Adoption::Stale);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn stale_analysis_never_replaces_published_diagnostics() {
    let mut service = Session::default();
    let document_uri = uri("file:///stale-diagnostics.adoc");
    let stale_job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": "trailing  \n"
            }
        })))
        .pop()
        .expect("stale job");
    let current_job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 2,
                "text": "current\n"
            }
        })))
        .pop()
        .expect("current job");
    adopt(&mut service, current_job);

    let stale_analysis = analyze_document_input(&stale_job, &adocweave::NeverCancel);
    assert_eq!(service.adopt(&stale_job, stale_analysis), Adoption::Stale);

    let published = service.diagnostics(&document_uri).expect("diagnostics");
    assert_eq!(published.version, Some(2));
    assert!(published.diagnostics.is_empty());
}

#[test]
fn rejected_change_keeps_protocol_input_for_the_next_incremental_change() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-oversized-open-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\nmax-files = 8\nmax-total-bytes = 8\nmax-resource-bytes = 4\n",
    )
    .expect("configuration");
    let document_path = root.join("document.adoc");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    let accepted = service.begin_open(typed(json!({
        "textDocument": {
            "uri": document_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "old"
        }
    })));
    assert_eq!(accepted.len(), 1);
    let stale_job = accepted.into_iter().next().expect("initial job");
    let stale_result = analyze_document_input(&stale_job, &adocweave::NeverCancel);

    let rejected = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "oversized"}]
        })))
        .expect("workspace rejection is a current document input");

    assert_eq!(rejected.len(), 1);
    assert_eq!(
        rejected[0]
            .project_problem
            .as_ref()
            .expect("workspace input problem")
            .code,
        "workspace-input-error"
    );
    assert!(stale_job.cancellation.is_cancelled());
    assert_eq!(service.adopt(&stale_job, stale_result), Adoption::Stale);
    let current = service
        .documents
        .get(document_uri.as_str())
        .expect("rejected document input");
    assert_eq!(current.document_input.revision.version, 2);
    assert_eq!(current.document_input.source.as_ref(), "oversized");
    adopt(
        &mut service,
        rejected.into_iter().next().expect("rejected analysis"),
    );
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code
            == Some(lsp::NumberOrString::String(
                "workspace-input-error".to_owned(),
            ))
            && diagnostic.message.contains("retained resource byte")
    }));

    let recovered = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 3},
            "contentChanges": [{
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 9}
                },
                "text": "new"
            }]
        })))
        .expect("incremental recovery uses the rejected version");
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].project_problem.is_none());
    adopt(
        &mut service,
        recovered.into_iter().next().expect("recovered analysis"),
    );
    let current = service
        .documents
        .get(document_uri.as_str())
        .expect("recovered document");
    assert_eq!(current.document_input.revision.version, 3);
    assert_eq!(current.document_input.source.as_ref(), "new");
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().all(|diagnostic| {
        diagnostic.code
            != Some(lsp::NumberOrString::String(
                "workspace-input-error".to_owned(),
            ))
    }));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn rejected_overlay_survives_dependent_and_configuration_reanalysis_until_retry() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-rejected-overlay-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\nmax-files = 8\nmax-total-bytes = 1024\nmax-resource-bytes = 32\n",
    )
    .expect("configuration");
    let document_path = root.join("root.adoc");
    let include_path = root.join("part.adoc");
    let initial = "include::part.adoc[]\n";
    fs::write(&document_path, initial).expect("document");
    fs::write(&include_path, "old\n").expect("include");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, initial);

    let rejected = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "x".repeat(64)}]
        })))
        .expect("rejected document change")
        .pop()
        .expect("rejected analysis job");
    assert!(rejected.project_problem.is_some());
    assert!(!rejected.cancellation.is_cancelled());

    fs::write(&include_path, "changed\n").expect("changed include");
    let dependent = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": include_uri, "type": 2}]
    })));
    assert!(dependent.jobs.is_empty());
    assert!(
        service
            .update_configuration(json!({"enabledRules": ["macro-boundary"]}))
            .expect("configuration update")
            .is_empty()
    );
    assert!(
        !rejected.cancellation.is_cancelled(),
        "unrelated inputs must not supersede the rejected document job"
    );
    let current = service
        .documents
        .get(document_uri.as_str())
        .expect("rejected document");
    assert_eq!(current.document_input.revision.version, 2);
    assert_eq!(current.document_input.source.len(), 64);
    assert!(service.begin_reanalysis_for_test(&document_uri).is_none());
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "workspace-input-error".to_owned()
                )))
    );
    adopt(&mut service, rejected);
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "workspace-input-error".to_owned()
                )))
    );

    let recovered = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 3},
            "contentChanges": [{"text": initial}]
        })))
        .expect("recovered document change");
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].project_context.is_some());
    assert!(recovered[0].project_problem.is_none());
    for job in recovered {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "workspace-input-error".to_owned()
                )))
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn valid_open_and_change_use_the_coherent_workspace_during_a_scan_error() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-open-after-scan-error-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    fs::write(&document_path, "disk\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    assert!(
        service
            .workspace_scan_failed("workspace scan worker failed".to_owned())
            .is_empty()
    );

    let opened = service.begin_open(typed(json!({
        "textDocument": {
            "uri": document_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "opened\n"
        }
    })));
    assert_eq!(opened.len(), 1);
    assert!(opened[0].project_context.is_some());
    assert!(opened[0].project_problem.is_none());
    for job in opened {
        adopt(&mut service, job);
    }

    let changed = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "changed\n"}]
        })))
        .expect("document change");
    assert_eq!(changed.len(), 1);
    assert!(changed[0].project_context.is_some());
    assert!(changed[0].project_problem.is_none());
    for job in changed {
        adopt(&mut service, job);
    }
    assert_eq!(
        service
            .workspace_resource(&document_uri)
            .expect("current overlay")
            .as_ref(),
        "changed\n"
    );
    let failures = service
        .diagnostics(&document_uri)
        .expect("diagnostics")
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.message.contains("workspace scan worker failed"))
        .count();
    assert_eq!(failures, 1);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn did_open_outside_configured_roots_keeps_input_until_close() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-outside-open-{unique}"));
    let docs = root.join("docs");
    let other = root.join("other");
    fs::create_dir_all(&docs).expect("docs");
    fs::create_dir_all(&other).expect("other");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\nroots = [\"docs\"]\n",
    )
    .expect("configuration");
    let accepted_path = docs.join("accepted.adoc");
    let rejected_path = other.join("rejected.adoc");
    fs::write(&accepted_path, "accepted").expect("accepted source");
    fs::write(&rejected_path, "rejected").expect("rejected source");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let accepted_uri = lsp::Url::from_file_path(&accepted_path).expect("accepted URI");
    let rejected_uri = lsp::Url::from_file_path(&rejected_path).expect("rejected URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    let jobs = service.begin_open(typed(json!({
        "textDocument": {
            "uri": accepted_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "accepted"
        }
    })));
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        adopt(&mut service, job);
    }

    let rejected = service.begin_open(typed(json!({
        "textDocument": {
            "uri": rejected_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "open"
        }
    })));

    assert_eq!(rejected.len(), 1);
    let cancellation = rejected[0].cancellation.clone();
    for job in rejected {
        adopt(&mut service, job);
    }
    let rejected_document = service
        .documents
        .get(rejected_uri.as_str())
        .expect("open document outside configured roots");
    assert_eq!(rejected_document.document_input.revision.version, 1);
    assert_eq!(rejected_document.document_input.source.as_ref(), "open");
    let diagnostics = service.diagnostics(&rejected_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code
            == Some(lsp::NumberOrString::String(
                "workspace-input-error".to_owned(),
            ))
            && diagnostic
                .message
                .contains("outside configured resource roots")
    }));
    let outcome = service.close(&rejected_uri);
    assert!(outcome.closed);
    assert!(outcome.reanalysis_jobs.is_empty());
    assert!(cancellation.is_cancelled());
    assert!(service.documents.get(rejected_uri.as_str()).is_none());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn workspace_folders_null_does_not_fall_back_to_legacy_root_uri() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-workspace-null-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("part.adoc"), "included\n").expect("part");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(root.join("root.adoc")).expect("document URI");
    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "rootUri": root_uri,
        "workspaceFolders": null,
        "capabilities": {"workspace": {"workspaceFolders": true}}
    }));
    initialize_with_params(&mut service, params);
    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );

    assert!(service.workspace_roots().is_empty());
    let analysis = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("the document parent remains usable without a workspace folder");
    assert!(analysis.analysis.source().contains("included"));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn legacy_root_path_is_used_only_when_root_uri_is_null() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-root-path-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("part.adoc"), "included\n").expect("part");
    let document_uri = lsp::Url::from_file_path(root.join("root.adoc")).expect("document URI");
    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "rootPath": root,
        "rootUri": null,
        "capabilities": {}
    }));
    initialize_with_params(&mut service, params);
    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );

    assert!(
        service
            .documents
            .get(document_uri.as_str())
            .expect("document")
            .expanded_analysis()
            .is_some()
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn workspace_folder_changes_rebuild_roots_and_preserve_open_overlays() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-workspace-change-{unique}"));
    let retained = base.join("retained");
    let removed = base.join("removed");
    let added = base.join("added");
    for root in [&retained, &removed, &added] {
        fs::create_dir_all(root).expect("workspace");
    }
    fs::write(retained.join("part.adoc"), "disk\n").expect("part");
    let retained_uri = lsp::Url::from_directory_path(&retained).expect("retained URI");
    let removed_uri = lsp::Url::from_directory_path(&removed).expect("removed URI");
    let added_uri = lsp::Url::from_directory_path(&added).expect("added URI");
    let document_uri = lsp::Url::from_file_path(retained.join("root.adoc")).expect("document URI");
    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "workspaceFolders": [
            {"uri": retained_uri, "name": "retained"},
            {"uri": removed_uri, "name": "removed"}
        ],
        "capabilities": {"workspace": {"workspaceFolders": true}}
    }));
    let result = initialize_with_params(&mut service, params);
    let value = serde_json::to_value(result).expect("initialize result");
    assert_eq!(
        value["capabilities"]["workspace"]["workspaceFolders"]["supported"],
        true
    );
    assert_eq!(
        value["capabilities"]["workspace"]["workspaceFolders"]["changeNotifications"],
        true
    );
    open(
        &mut service,
        document_uri.as_str(),
        3,
        "include::part.adoc[]\n\noverlay\n",
    );
    let stale_job = service
        .begin_reanalysis_for_test(&document_uri)
        .expect("pending analysis");
    let stale_result = analyze_document_input(&stale_job, &adocweave::NeverCancel);

    let _jobs = service.workspace_folders_changed(typed(json!({
        "event": {
            "removed": [{"uri": removed_uri, "name": "removed"}],
            "added": [{"uri": added_uri, "name": "added"}]
        }
    })));
    assert!(stale_job.cancellation.is_cancelled());
    assert_eq!(service.adopt(&stale_job, stale_result), Adoption::Stale);
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(scan).jobs;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].document_input.revision.version, 3);
    assert!(jobs[0].document_input.source.contains("overlay"));
    assert_eq!(
        jobs[0]
            .project_context
            .as_ref()
            .expect("retained workspace")
            .root_text()
            .expect("root resource")
            .as_ref(),
        "include::part.adoc[]\n\noverlay\n"
    );
    fs::remove_dir_all(base).expect("cleanup");
}

#[test]
fn removing_a_workspace_folder_does_not_fail_the_retained_folder() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-workspace-removal-{unique}"));
    let retained = base.join("retained");
    let removed = base.join("removed");
    for root in [&retained, &removed] {
        fs::create_dir_all(root).expect("workspace");
        fs::write(root.join("root.adoc"), "disk\n").expect("document");
    }
    let retained_root = lsp::Url::from_directory_path(&retained).expect("retained root URI");
    let removed_root = lsp::Url::from_directory_path(&removed).expect("removed root URI");
    let retained_document =
        lsp::Url::from_file_path(retained.join("root.adoc")).expect("retained document URI");
    let removed_document =
        lsp::Url::from_file_path(removed.join("root.adoc")).expect("removed document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "workspaceFolders": [
                {"uri": retained_root, "name": "retained"},
                {"uri": removed_root, "name": "removed"}
            ],
            "capabilities": {"workspace": {"workspaceFolders": true}}
        })),
    );
    open(
        &mut service,
        retained_document.as_str(),
        1,
        "retained overlay\n",
    );
    open(
        &mut service,
        removed_document.as_str(),
        1,
        "removed overlay\n",
    );

    let _jobs = service.workspace_folders_changed(typed(json!({
        "event": {
            "removed": [{"uri": removed_root, "name": "removed"}],
            "added": []
        }
    })));
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(scan).jobs;

    let retained_job = jobs
        .iter()
        .find(|job| job.uri == retained_document.as_str())
        .expect("retained document reanalysis");
    assert!(
        retained_job.project_context.is_some(),
        "retained workspace problem: {:?}",
        retained_job.project_problem
    );
    assert!(retained_job.project_problem.is_none());
    let removed_job = jobs
        .iter()
        .find(|job| job.uri == removed_document.as_str())
        .expect("removed document reanalysis");
    assert!(removed_job.project_context.is_none());
    assert_eq!(
        removed_job
            .project_problem
            .as_ref()
            .expect("removed document problem")
            .code,
        "workspace-input-error"
    );
    assert_eq!(
        service
            .workspace_resource(&retained_document)
            .expect("retained workspace source")
            .as_ref(),
        "retained overlay\n"
    );

    let stale_job = removed_job;
    let stale_result = analyze_document_input(stale_job, &adocweave::NeverCancel);
    let changed = service
        .begin_change(typed(json!({
            "textDocument": {"uri": removed_document, "version": 2},
            "contentChanges": [{"text": "edited after removal\n"}]
        })))
        .expect("workspace rejection does not reject the LSP notification");

    assert_eq!(changed.len(), 1);
    assert!(stale_job.cancellation.is_cancelled());
    assert_eq!(service.adopt(stale_job, stale_result), Adoption::Stale);
    let current = service
        .documents
        .get(removed_document.as_str())
        .expect("removed-root document remains open");
    assert_eq!(current.document_input.revision.version, 2);
    assert_eq!(
        current.document_input.source.as_ref(),
        "edited after removal\n"
    );
    assert_eq!(
        changed[0]
            .project_problem
            .as_ref()
            .expect("removed-root input problem")
            .code,
        "workspace-input-error"
    );
    adopt(
        &mut service,
        changed.into_iter().next().expect("current rejected job"),
    );

    fs::remove_dir_all(base).expect("cleanup");
}

#[test]
fn workspace_configuration_updates_and_caps_debounce() {
    let mut service = Session::default();
    service
        .update_configuration(json!({"adocweave": {"debounceMs": 25}}))
        .expect("configuration");
    assert_eq!(service.debounce_ms(), 25);

    service
        .update_configuration(json!({"debounceMs": 50_000}))
        .expect("configuration");
    assert_eq!(service.debounce_ms(), 1_000);
    assert!(
        service
            .update_configuration(json!({"unknown": true}))
            .is_err()
    );
    assert!(
        service
            .update_configuration(json!({"enabledRules": ["unknown-rule"]}))
            .is_err()
    );
}

#[test]
fn project_configuration_is_shared_with_lsp_and_reloaded_by_generation() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-config-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    let config_path = root.join(adocweave_config::FILE_NAME);
    let source = "日😀xref:guide.adoc[Guide]\n\nSecond\n";
    fs::write(&document_path, source).expect("document");
    fs::write(
        &config_path,
        include_str!("../../../../fixtures/config/shared-v2/.adocweave.toml"),
    )
    .expect("configuration");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("config URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, source);
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
            && diagnostic.severity == Some(lsp::DiagnosticSeverity::ERROR)
    }));
    let edits = service
        .formatting(&document_uri)
        .expect("formatting")
        .expect("formatting response");
    assert_eq!(
        apply_edits(source, &edits),
        "日😀xref:guide.adoc[Guide]\r\n\r\nSecond"
    );

    fs::write(
        &config_path,
        "schema-version = 2\n[lint.rules.macro-boundary]\nenabled = false\n",
    )
    .expect("updated configuration");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": config_uri, "type": 2}]
            })))
            .jobs
            .is_empty()
    );
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(scan).jobs;
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        adopt(&mut service, job);
    }
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().all(|diagnostic| {
        diagnostic.code != Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
    }));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn configuration_watch_does_not_restore_open_overlay_outside_resource_roots() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-root-authority-{unique}"));
    let docs = root.join("docs");
    fs::create_dir_all(&docs).expect("workspace");
    let document_path = root.join("outside.adoc");
    let config_path = root.join(adocweave_config::FILE_NAME);
    fs::write(&document_path, "disk").expect("document");
    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\nroots = [\".\"]\nmax-files = 8\nmax-total-bytes = 64\nmax-resource-bytes = 64\n",
    )
    .expect("configuration");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("config URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, "open overlay");

    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\nroots = [\"docs\"]\nmax-files = 8\nmax-total-bytes = 64\nmax-resource-bytes = 64\n",
    )
    .expect("narrowed configuration");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": config_uri, "type": 2}]
            })))
            .jobs
            .is_empty()
    );
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(scan).jobs;

    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].project_context.is_none());
    assert_eq!(
        jobs[0]
            .project_problem
            .as_ref()
            .expect("fail-closed workspace problem")
            .code,
        "workspace-input-error"
    );
    adopt(&mut service, jobs.into_iter().next().expect("reanalysis"));
    assert!(service.begin_reanalysis_for_test(&document_uri).is_none());
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code
            == Some(lsp::NumberOrString::String(
                "workspace-input-error".to_owned(),
            ))
    }));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn project_configuration_bounds_lsp_diagnostics_before_protocol_projection() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-config-limit-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    let source = "long \n*x\n";
    fs::write(&document_path, source).expect("document");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[lint]\nmax-line-length = 4\nmax-diagnostics = 1\n",
    )
    .expect("configuration");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, source);

    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert_eq!(diagnostics.diagnostics.len(), 1);
    assert_eq!(
        diagnostics.diagnostics[0].code,
        Some(lsp::NumberOrString::String(
            "trailing-whitespace".to_owned()
        ))
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn each_workspace_folder_uses_its_own_project_configuration() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-lsp-multi-config-{unique}"));
    let enabled_root = base.join("enabled");
    let disabled_root = base.join("disabled");
    fs::create_dir_all(&enabled_root).expect("enabled workspace");
    fs::create_dir_all(&disabled_root).expect("disabled workspace");
    fs::write(
        enabled_root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[lint.rules.macro-boundary]\nenabled = true\nseverity = \"error\"\n",
    )
    .expect("enabled configuration");
    fs::write(
        disabled_root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[lint.rules.macro-boundary]\nenabled = false\n",
    )
    .expect("disabled configuration");
    let source = "日😀xref:guide.adoc[Guide]\n";
    let enabled_path = enabled_root.join("root.adoc");
    let disabled_path = disabled_root.join("root.adoc");
    fs::write(&enabled_path, source).expect("enabled document");
    fs::write(&disabled_path, source).expect("disabled document");
    let enabled_root_uri = lsp::Url::from_directory_path(&enabled_root).expect("enabled root URI");
    let disabled_root_uri =
        lsp::Url::from_directory_path(&disabled_root).expect("disabled root URI");
    let enabled_uri = lsp::Url::from_file_path(&enabled_path).expect("enabled document URI");
    let disabled_uri = lsp::Url::from_file_path(&disabled_path).expect("disabled document URI");

    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "workspaceFolders": [
                {"uri": enabled_root_uri, "name": "enabled"},
                {"uri": disabled_root_uri, "name": "disabled"}
            ],
            "capabilities": {"workspace": {"workspaceFolders": true}}
        })),
    );
    open(&mut service, enabled_uri.as_str(), 1, source);
    open(&mut service, disabled_uri.as_str(), 1, source);

    let enabled = service
        .diagnostics(&enabled_uri)
        .expect("enabled diagnostics");
    assert!(enabled.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
            && diagnostic.severity == Some(lsp::DiagnosticSeverity::ERROR)
    }));
    let disabled = service
        .diagnostics(&disabled_uri)
        .expect("disabled diagnostics");
    assert!(disabled.diagnostics.iter().all(|diagnostic| {
        diagnostic.code != Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
    }));

    fs::remove_dir_all(base).expect("cleanup");
}

#[test]
fn invalid_project_configuration_does_not_fall_back_to_default_analysis() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-invalid-config-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 1\n",
    )
    .expect("obsolete configuration");
    fs::write(&document_path, "trailing \n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    let jobs = service.begin_open(typed(json!({
        "textDocument": {
            "uri": document_uri,
            "languageId": "asciidoc",
            "version": 1,
            "text": "trailing \n"
        }
    })));
    assert_eq!(jobs.len(), 1, "the workspace error must be publishable");
    assert!(jobs[0].project_context.is_none());
    assert!(jobs[0].project_problem.is_some());
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(service.documents.get(document_uri.as_str()).is_some());

    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code
            == Some(lsp::NumberOrString::String(
                "workspace-resource-error".to_owned(),
            ))
    }));
    assert!(diagnostics.diagnostics.iter().all(|diagnostic| {
        diagnostic.code
            != Some(lsp::NumberOrString::String(
                "trailing-whitespace".to_owned(),
            ))
    }));
    assert!(
        service
            .formatting(&document_uri)
            .expect("formatting")
            .expect("response")
            .is_empty()
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn invalidated_project_configuration_clears_old_feature_views() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-invalidated-config-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    let config_path = root.join(adocweave_config::FILE_NAME);
    let source = "xref:other.adoc[Other]\ntrailing  \n";
    fs::write(&document_path, source).expect("document");
    fs::write(&config_path, "schema-version = 2\n").expect("configuration");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("config URI");
    let mut service = Session::default();
    initialize(&mut service, &["utf-16"]);
    let _jobs = service.workspace_folders_changed(typed(json!({
        "event": {"added": [{"uri": root_uri, "name": "root"}], "removed": []}
    })));
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let _ = service.apply_workspace_scan(scan);
    open(&mut service, document_uri.as_str(), 1, source);
    assert!(service.documents.snapshot(document_uri.as_str()).is_some());

    fs::write(&config_path, "schema-version = 99\n").expect("invalid configuration");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": config_uri, "type": 2}]
            })))
            .jobs
            .is_empty()
    );
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(scan).jobs;

    assert_eq!(jobs.len(), 1);
    assert!(service.documents.snapshot(document_uri.as_str()).is_none());
    assert!(
        service
            .hover(&document_uri, lsp::Position::new(0, 6))
            .expect("hover")
            .is_none()
    );
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().all(|diagnostic| {
        diagnostic.code
            != Some(lsp::NumberOrString::String(
                "trailing-whitespace".to_owned(),
            ))
    }));
    for job in jobs {
        assert!(job.project_problem.is_some());
        adopt(&mut service, job);
    }
    assert!(service.documents.snapshot(document_uri.as_str()).is_none());
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn watched_resource_failure_is_published_and_cleared_after_recovery() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-watch-recovery-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\".\"]\n",
    )
    .expect("configuration");
    let document_path = root.join("root.adoc");
    let include_path = root.join("part.adoc");
    let unrelated_path = root.join("unrelated.adoc");
    fs::write(&document_path, "include::part.adoc[]\n").expect("document");
    fs::write(&include_path, "part\n").expect("include");
    fs::write(&unrelated_path, "unrelated\n").expect("unrelated document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let unrelated_uri = lsp::Url::from_file_path(&unrelated_path).expect("unrelated URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );

    fs::write(&include_path, [0xff]).expect("invalid include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 2}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1, "watch failure must be published");
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("failure diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "workspace-resource-error".to_owned()
                )))
    );

    fs::write(&unrelated_path, "changed\n").expect("changed unrelated document");
    for job in service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": unrelated_uri, "type": 2}]
        })))
        .jobs
    {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("unrelated change diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "workspace-resource-error".to_owned()
                ))),
        "a successful notification for another URI must not clear the failure"
    );

    fs::write(&include_path, "restored\n").expect("restored include");
    let jobs = service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [{"uri": include_uri, "type": 2}]
        })))
        .jobs;
    assert_eq!(jobs.len(), 1, "watch recovery must clear the failure");
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("recovered diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "workspace-resource-error".to_owned()
                )))
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn watched_file_batch_applies_only_the_final_event_for_each_uri() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-watch-coalesce-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    let include_path = root.join("part.adoc");
    fs::write(&document_path, "include::part.adoc[]\n").expect("document");
    fs::write(&include_path, "part\n").expect("include");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );

    fs::remove_file(&include_path).expect("remove include");
    for job in service
        .workspace_files_changed_with_journal(typed(json!({
            "changes": [
                {"uri": include_uri, "type": 2},
                {"uri": include_uri, "type": 3}
            ]
        })))
        .jobs
    {
        adopt(&mut service, job);
    }

    assert!(
        service
            .diagnostics(&document_uri)
            .expect("diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "workspace-resource-error".to_owned()
                ))),
        "the discarded changed event must not leave a watch read failure"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn stricter_resource_plan_invalidates_the_rejected_open_overlay() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-overlay-plan-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    let config_path = root.join(adocweave_config::FILE_NAME);
    fs::write(&document_path, "disk\n").expect("document");
    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 16\nmax-resource-bytes = 16\n",
    )
    .expect("configuration");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let config_uri = lsp::Url::from_file_path(&config_path).expect("config URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null, "rootUri": root_uri, "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, "`open`\n");
    fs::write(
        &config_path,
        "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 8\nmax-resource-bytes = 8\n",
    )
    .expect("stricter configuration");

    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": config_uri, "type": 2}]
            })))
            .jobs
            .is_empty()
    );
    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(scan).jobs;

    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].project_context.is_none());
    assert!(
        jobs[0]
            .project_problem
            .as_ref()
            .expect("input error")
            .message
            .contains("retained resource byte")
    );
    assert!(service.documents.snapshot(document_uri.as_str()).is_none());
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(service.documents.snapshot(document_uri.as_str()).is_none());
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code
            == Some(lsp::NumberOrString::String(
                "workspace-input-error".to_owned(),
            ))
    }));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn workspace_configuration_reanalyzes_open_documents_with_enabled_rules() {
    let mut service = Session::default();
    open(
        &mut service,
        "file:///configured-rule.adoc",
        1,
        "日😀xref:guide.adoc[Guide]\n",
    );
    let document = uri("file:///configured-rule.adoc");
    assert!(
        service
            .diagnostics(&document)
            .expect("default diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| {
                diagnostic.code != Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
            })
    );

    let jobs = service
        .update_configuration(json!({"enabledRules": ["macro-boundary"]}))
        .expect("configuration");
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document)
            .expect("configured diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic.code == Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
            })
    );

    let jobs = service
        .update_configuration(json!({"enabledRules": []}))
        .expect("disabled configuration");
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document)
            .expect("disabled diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| {
                diagnostic.code != Some(lsp::NumberOrString::String("macro-boundary".to_owned()))
            })
    );
}

#[test]
fn workspace_include_analysis_uses_versioned_resources_and_projects_diagnostics() {
    let mut service = Session::default();
    open(&mut service, "file:///book/part.adoc", 3, "==Part\n");
    open(
        &mut service,
        "file:///book/root.adoc",
        1,
        "= Root\n\ninclude::part.adoc[]\n",
    );

    let root = service
        .documents
        .get("file:///book/root.adoc")
        .expect("root");
    let workspace = root.expanded_analysis().expect("expanded analysis");
    assert!(workspace.analysis.source().contains("==Part"));
    assert_eq!(
        workspace.resource_versions.get("file:///book/part.adoc"),
        Some(&3)
    );
    let links = service
        .document_links(&uri("file:///book/root.adoc"))
        .expect("document links")
        .expect("links");
    assert!(links.iter().any(|link| {
        link.target.as_ref().map(lsp::Url::as_str) == Some("file:///book/part.adoc")
            && link.range.start == lsp::Position::new(2, 9)
    }));
    let definition = service
        .definition(&uri("file:///book/root.adoc"), lsp::Position::new(2, 10))
        .expect("definition")
        .expect("include definition");
    let lsp::GotoDefinitionResponse::Scalar(definition) = definition else {
        panic!("scalar include definition");
    };
    assert_eq!(definition.uri.as_str(), "file:///book/part.adoc");

    let diagnostics = service
        .diagnostics(&uri("file:///book/part.adoc"))
        .expect("diagnostics");
    assert_eq!(
        diagnostics
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "heading-marker-space".to_owned()
                )))
            .count(),
        1,
        "direct and projected diagnostics are deduplicated: {:#?}",
        diagnostics.diagnostics
    );

    let root_generation = service
        .documents
        .get("file:///book/root.adoc")
        .expect("root")
        .document_input
        .revision
        .generation;
    assert!(
        change(
            &mut service,
            "file:///book/part.adoc",
            4,
            json!([{"text": "== Part\n"}]),
        )
        .expect("change")
    );
    let reanalyzed = service
        .documents
        .get("file:///book/root.adoc")
        .expect("root");
    assert!(reanalyzed.document_input.revision.generation > root_generation);
    assert!(reanalyzed.expanded_analysis().is_some());
}

#[test]
fn missing_include_is_reported_as_a_project_diagnostic_at_the_directive() {
    let mut service = Session::default();
    let document_uri = uri("file:///book/root.adoc");
    open(
        &mut service,
        document_uri.as_str(),
        1,
        "= Root\n\ninclude::missing.adoc[]\n",
    );

    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    let problem = diagnostics
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.source.as_deref() == Some("adocweave-project"))
        .expect("project diagnostic");
    assert_eq!(
        problem.code,
        Some(lsp::NumberOrString::String("missing-resource".to_owned()))
    );
    assert_eq!(problem.range.start.line, 2);
    assert!(
        service.documents.snapshot(document_uri.as_str()).is_some(),
        "the primary analysis remains available"
    );
    assert!(
        service
            .document_symbols(&document_uri)
            .expect("document symbols")
            .is_some(),
        "primary-source language features remain available"
    );
}

#[test]
fn include_outside_the_project_is_rejected_without_discarding_primary_analysis() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-lsp-project-authority-{unique}"));
    let project = base.join("project");
    fs::create_dir_all(&project).expect("project directory");
    fs::write(base.join("outside.adoc"), "outside\n").expect("outside document");
    let project_uri = lsp::Url::from_directory_path(&project).expect("project URI");
    let document_uri = lsp::Url::from_file_path(project.join("guide.adoc")).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": project_uri,
            "capabilities": {}
        })),
    );

    open(
        &mut service,
        document_uri.as_str(),
        1,
        "= Guide\n\ninclude::../outside.adoc[]\n",
    );

    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == Some(lsp::NumberOrString::String("unsafe-target".to_owned()))
    }));
    assert!(service.documents.snapshot(document_uri.as_str()).is_some());
    assert!(
        service
            .document_symbols(&document_uri)
            .expect("document symbols")
            .is_some()
    );

    fs::remove_dir_all(base).expect("cleanup");
}

#[test]
fn rejected_nested_include_is_reported_at_its_own_directive() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!("adocweave-lsp-nested-authority-{unique}"));
    let project = base.join("project");
    fs::create_dir_all(&project).expect("project directory");
    let chapter_path = project.join("chapter.adoc");
    fs::write(&chapter_path, "= Chapter\n\ninclude::../outside.adoc[]\n")
        .expect("chapter document");
    fs::write(base.join("outside.adoc"), "outside\n").expect("outside document");
    let project_uri = lsp::Url::from_directory_path(&project).expect("project URI");
    let document_uri = lsp::Url::from_file_path(project.join("guide.adoc")).expect("document URI");
    let chapter_uri = lsp::Url::from_file_path(&chapter_path).expect("chapter URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": project_uri,
            "capabilities": {}
        })),
    );

    open(
        &mut service,
        document_uri.as_str(),
        1,
        "= Guide\n\ninclude::chapter.adoc[]\n",
    );

    let diagnostics = service
        .diagnostics(&chapter_uri)
        .expect("chapter diagnostics");
    let problem = diagnostics
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.code == Some(lsp::NumberOrString::String("unsafe-target".to_owned()))
        })
        .expect("rejected include diagnostic");
    assert_eq!(problem.range.start.line, 2);

    fs::remove_dir_all(base).expect("cleanup");
}

#[test]
fn replacing_or_closing_a_root_republishes_diagnostics_for_old_includes() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-old-include-diagnostics-{unique}"));
    fs::create_dir_all(&root).expect("project directory");
    let document_path = root.join("guide.adoc");
    let include_path = root.join("part.adoc");
    fs::write(&document_path, "include::part.adoc[]\n").expect("primary document");
    fs::write(&include_path, "= Part\n").expect("include document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("project URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );
    let mut jobs = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "= Guide\n"}]
        })))
        .expect("change");
    let mut job = jobs.pop().expect("replacement analysis");
    let project = job.prepared_request.take().expect("project request");
    let result = adocweave_project::process(project.request, job.cancellation.as_ref())
        .expect("project analysis");
    let published = service.adopt_project_result(&job, result, project.source_index);
    assert!(
        published.contains(&include_uri.to_string()),
        "the old include must receive an empty diagnostic publication"
    );

    open(
        &mut service,
        document_uri.as_str(),
        3,
        "include::part.adoc[]\n",
    );
    let outcome = service.close(&document_uri);
    assert!(outcome.closed);
    assert!(outcome.diagnostic_uris.contains(include_uri.as_str()));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn local_target_diagnostics_are_published_for_each_source_and_follow_file_creation() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-local-target-watch-{unique}"));
    fs::create_dir_all(&root).expect("project directory");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    )
    .expect("configuration");
    let document_path = root.join("guide.adoc");
    let include_path = root.join("part.adoc");
    let root_asset_path = root.join("root.png");
    let include_asset_path = root.join("part.png");
    fs::write(
        &document_path,
        "include::part.adoc[]\n\nimage::root.png[]\n",
    )
    .expect("primary document");
    fs::write(&include_path, "image::part.png[]\n").expect("include document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("project URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n\nimage::root.png[]\n",
    );
    for uri in [&document_uri, &include_uri] {
        assert!(
            service
                .diagnostics(uri)
                .expect("diagnostics")
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code
                    == Some(lsp::NumberOrString::String(
                        "local-target-missing".to_owned()
                    )))
        );
    }

    fs::write(&root_asset_path, [0x89, b'P', b'N', b'G', 0xff]).expect("root asset");
    let root_asset_uri = lsp::Url::from_file_path(&root_asset_path).expect("root asset URI");
    let outcome = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": root_asset_uri, "type": 1}]
    })));
    assert_eq!(outcome.jobs.len(), 1);
    for job in outcome.jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("root diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| {
                !matches!(
                    diagnostic.code,
                    Some(lsp::NumberOrString::String(ref code))
                        if code == "local-target-missing" || code == "workspace-resource-error"
                )
            })
    );
    assert!(
        service
            .diagnostics(&include_uri)
            .expect("include diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "local-target-missing".to_owned()
                )))
    );

    fs::write(&include_asset_path, b"image").expect("include asset");
    let include_asset_uri =
        lsp::Url::from_file_path(&include_asset_path).expect("include asset URI");
    let outcome = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": include_asset_uri, "type": 1}]
    })));
    assert_eq!(outcome.jobs.len(), 1);
    for job in outcome.jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&include_uri)
            .expect("include diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "local-target-missing".to_owned()
                )))
    );

    fs::remove_file(&root_asset_path).expect("delete root asset");
    let outcome = service.workspace_files_changed_with_journal(typed(json!({
        "changes": [{"uri": root_asset_uri, "type": 3}]
    })));
    assert_eq!(outcome.jobs.len(), 1);
    for job in outcome.jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("root diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "local-target-missing".to_owned()
                )))
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn local_target_change_before_dependency_registration_retries_the_project() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-local-target-race-{unique}"));
    fs::create_dir_all(&root).expect("project directory");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    )
    .expect("configuration");
    let document_path = root.join("guide.adoc");
    let asset_path = root.join("asset.png");
    fs::write(&document_path, "image::asset.png[]\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("project URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let asset_uri = lsp::Url::from_file_path(&asset_path).expect("asset URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    let mut job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": "image::asset.png[]\n"
            }
        })))
        .pop()
        .expect("analysis job");
    let prepared = job.prepared_request.take().expect("project request");
    let result = adocweave_project::process(prepared.request, job.cancellation.as_ref())
        .expect("project analysis");

    fs::write(&asset_path, [0x89, b'P', b'N', b'G', 0xff]).expect("asset");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": asset_uri, "type": 1}]
            })))
            .jobs
            .is_empty(),
        "the dependency is not registered until processing completes"
    );
    service.record_project_result_dependencies(&job, &result, &prepared.source_index);
    assert!(!crate::service::project_observations_are_current(
        &result,
        &prepared.observation_access,
        &adocweave::NeverCancel,
    ));

    let retry = service
        .retry_project_analysis(&job)
        .expect("changed observation retries the current document");
    adopt(&mut service, retry);
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "local-target-missing".to_owned()
                )))
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn non_adoc_include_change_before_dependency_registration_retries_the_project() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-text-include-race-{unique}"));
    fs::create_dir_all(&root).expect("project directory");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\n",
    )
    .expect("configuration");
    let document_path = root.join("guide.adoc");
    let include_path = root.join("fragment.txt");
    fs::write(&document_path, "include::fragment.txt[]\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("project URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    let mut job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": "include::fragment.txt[]\n"
            }
        })))
        .pop()
        .expect("analysis job");
    let prepared = job.prepared_request.take().expect("project request");
    let result = adocweave_project::process(prepared.request, job.cancellation.as_ref())
        .expect("project analysis");

    fs::write(&include_path, "included text\n").expect("include");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": include_uri, "type": 1}]
            })))
            .jobs
            .is_empty()
    );
    service.record_project_result_dependencies(&job, &result, &prepared.source_index);
    assert!(!crate::service::project_observations_are_current(
        &result,
        &prepared.observation_access,
        &adocweave::NeverCancel,
    ));

    let retry = service
        .retry_project_analysis(&job)
        .expect("changed observation retries the current document");
    adopt(&mut service, retry);
    assert!(
        service
            .documents
            .get(document_uri.as_str())
            .and_then(|document| document.expanded_analysis())
            .is_some_and(|analysis| analysis.analysis.source().contains("included text"))
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn stale_project_result_cannot_replace_current_dependencies() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-stale-dependencies-{unique}"));
    fs::create_dir_all(&root).expect("project directory");
    fs::write(
        root.join(adocweave_config::FILE_NAME),
        "schema-version = 2\n[resources]\ninclude = true\n",
    )
    .expect("configuration");
    let document_path = root.join("guide.adoc");
    let old_include_path = root.join("old.txt");
    let current_include_path = root.join("current.txt");
    fs::write(&document_path, "include::old.txt[]\n").expect("document");
    fs::write(&old_include_path, "old\n").expect("old include");
    fs::write(&current_include_path, "current\n").expect("current include");
    let root_uri = lsp::Url::from_directory_path(&root).expect("project URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let old_include_uri = lsp::Url::from_file_path(&old_include_path).expect("old include URI");
    let current_include_uri =
        lsp::Url::from_file_path(&current_include_path).expect("current include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    let mut old_job = service
        .begin_open(typed(json!({
            "textDocument": {
                "uri": document_uri,
                "languageId": "asciidoc",
                "version": 1,
                "text": "include::old.txt[]\n"
            }
        })))
        .pop()
        .expect("old job");
    let old_project = old_job.prepared_request.take().expect("old request");
    let old_result = adocweave_project::process(old_project.request, old_job.cancellation.as_ref())
        .expect("old result");

    let mut current_job = service
        .begin_change(typed(json!({
            "textDocument": {"uri": document_uri, "version": 2},
            "contentChanges": [{"text": "include::current.txt[]\n"}]
        })))
        .expect("change")
        .pop()
        .expect("current job");
    let current_project = current_job
        .prepared_request
        .take()
        .expect("current request");
    let current_result =
        adocweave_project::process(current_project.request, current_job.cancellation.as_ref())
            .expect("current result");

    assert!(service.record_project_result_dependencies(
        &current_job,
        &current_result,
        &current_project.source_index,
    ));
    assert!(!service.record_project_result_dependencies(
        &old_job,
        &old_result,
        &old_project.source_index,
    ));

    fs::write(&old_include_path, "old changed\n").expect("change old include");
    assert!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": old_include_uri, "type": 2}]
            })))
            .jobs
            .is_empty()
    );
    fs::write(&current_include_path, "current changed\n").expect("change current include");
    assert_eq!(
        service
            .workspace_files_changed_with_journal(typed(json!({
                "changes": [{"uri": current_include_uri, "type": 2}]
            })))
            .jobs
            .len(),
        1
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn closing_an_include_overlay_reanalyzes_the_parent_with_disk_contents() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-lsp-close-include-{unique}"));
    fs::create_dir_all(&root).expect("project directory");
    let document_path = root.join("guide.adoc");
    let include_path = root.join("part.adoc");
    fs::write(&document_path, "include::part.adoc[]\n").expect("primary document");
    fs::write(&include_path, "disk contents\n").expect("include document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("project URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let include_uri = lsp::Url::from_file_path(&include_path).expect("include URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );

    open(
        &mut service,
        document_uri.as_str(),
        1,
        "include::part.adoc[]\n",
    );
    open(&mut service, include_uri.as_str(), 1, "open overlay\n");
    let with_overlay = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("analysis with open include");
    assert!(with_overlay.analysis.source().contains("open overlay"));

    let outcome = service.close(&include_uri);
    assert!(outcome.closed);
    assert_eq!(
        outcome.reanalysis_jobs.len(),
        1,
        "the including document is reanalyzed"
    );
    for job in outcome.reanalysis_jobs {
        adopt(&mut service, job);
    }
    let from_disk = service
        .documents
        .get(document_uri.as_str())
        .and_then(|document| document.expanded_analysis())
        .expect("analysis with disk include");
    assert!(from_disk.analysis.source().contains("disk contents"));
    assert!(!from_disk.analysis.source().contains("open overlay"));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn document_updates_are_ordered_and_stale_versions_are_ignored() {
    let mut service = Session::default();
    open(&mut service, "file:///a.adoc", 2, "= A");
    open(&mut service, "file:///b.adoc", 2, "= B");

    assert!(
        !change(
            &mut service,
            "file:///a.adoc",
            1,
            json!([{"text": "stale"}])
        )
        .expect("stale change")
    );
    assert_eq!(
        service
            .documents
            .get("file:///a.adoc")
            .expect("a")
            .analysis()
            .expect("analysis")
            .source(),
        "= A"
    );
    assert_eq!(
        service
            .documents
            .get("file:///b.adoc")
            .expect("b")
            .analysis()
            .expect("analysis")
            .source(),
        "= B"
    );
}

#[test]
fn incremental_changes_apply_sequentially_with_negotiated_positions() {
    let mut service = Session::default();
    open(&mut service, "file:///a.adoc", 1, "a😀c");
    assert!(
        change(
            &mut service,
            "file:///a.adoc",
            2,
            json!([
                {
                    "range": {
                        "start": {"line": 0, "character": 1},
                        "end": {"line": 0,"character": 3}
                    },
                    "text": "b"
                },
                {
                    "range": {
                        "start": {"line": 0, "character": 2},
                        "end": {"line": 0,"character": 3}
                    },
                    "text": "d"
                }
            ]),
        )
        .expect("incremental change")
    );
    assert_eq!(
        service
            .documents
            .get("file:///a.adoc")
            .expect("document")
            .analysis()
            .expect("analysis")
            .source(),
        "abd"
    );
}

#[test]
fn incremental_changes_preserve_crlf_line_boundaries() {
    let mut service = Session::default();
    open(&mut service, "file:///crlf.adoc", 1, "one\r\ntwo\r\n");
    assert!(
        change(
            &mut service,
            "file:///crlf.adoc",
            2,
            json!([{
                "range": {
                    "start": {"line": 1, "character": 0},
                    "end": {"line": 1, "character": 3}
                },
                "text": "second"
            }])
        )
        .expect("incremental change")
    );
    assert_eq!(
        service
            .documents
            .get("file:///crlf.adoc")
            .expect("document")
            .analysis()
            .expect("analysis")
            .source(),
        "one\r\nsecond\r\n"
    );
}

/// Planning a scan reads the roots without changing the service.
///
/// The walk runs on a worker while the event loop keeps answering requests, so
/// it must not touch state that the loop is free to change meanwhile. The
/// separation is the property under test: planning alone leaves the workspace
/// as it was, and only applying installs what was read.
#[test]
fn planning_a_workspace_scan_leaves_service_state_untouched() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root: PathBuf = std::env::temp_dir().join(format!("adocweave-detached-scan-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("found.adoc"), "= Found\n\n[[found]]\n== Found\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let found = lsp::Url::from_file_path(root.join("found.adoc")).expect("document URI");

    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "rootUri": root_uri,
        "capabilities": {}
    }));
    service.initialize(&params);

    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    assert!(
        service.workspace_analysis_count() == 0,
        "planning must not install what it read",
    );

    let jobs = service.apply_workspace_scan(scan).jobs;
    assert!(
        jobs.is_empty(),
        "no document is open, so nothing needs reanalysis",
    );
    assert!(service.workspace_resource(&found).is_some());

    fs::remove_dir_all(&root).expect("cleanup");
}

/// A document opened while the scan was running is not lost by applying it.
///
/// The read starts from the state at planning time. Overlaying the documents
/// open when the result lands, rather than the ones open when it started,
/// keeps an editor that opens a file immediately after initialization.
#[test]
fn a_document_opened_during_the_scan_survives_applying_it() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root: PathBuf = std::env::temp_dir().join(format!("adocweave-scan-race-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    fs::write(root.join("on-disk.adoc"), "= On disk\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let opened = lsp::Url::from_file_path(root.join("on-disk.adoc")).expect("document URI");

    let mut service = Session::default();
    let params = typed(json!({
        "processId": null,
        "rootUri": root_uri,
        "capabilities": {}
    }));
    service.initialize(&params);

    let scan = service.plan_workspace_scan(&adocweave::NeverCancel);
    // The client opens the file, with unsaved edits, before the walk lands.
    open(&mut service, opened.as_str(), 1, "= Edited in the editor\n");
    let jobs = service.apply_workspace_scan(scan).jobs;

    assert_eq!(jobs.len(), 1, "the open document is reanalyzed");
    assert_eq!(
        service
            .workspace_resource(&opened)
            .expect("open document")
            .as_ref(),
        "= Edited in the editor\n",
        "applying the scan must not replace the editor's text with disk text",
    );

    fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn a_scan_worker_failure_keeps_the_last_coherent_workspace_and_reports_it() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-scan-worker-failure-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let document_path = root.join("root.adoc");
    fs::write(&document_path, "= Root\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");

    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, "= Root\n");
    let previous = service
        .workspace_resource(&document_uri)
        .expect("workspace resource")
        .clone();

    let jobs = service.workspace_scan_failed("workspace scan worker failed: panic".to_owned());

    assert_eq!(jobs.len(), 1, "the open document must publish the failure");
    for job in jobs {
        adopt(&mut service, job);
    }
    assert_eq!(
        service
            .workspace_resource(&document_uri)
            .expect("retained workspace resource"),
        previous,
        "an internal worker failure must not replace the last coherent snapshot",
    );
    let diagnostics = service.diagnostics(&document_uri).expect("diagnostics");
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.code
            == Some(lsp::NumberOrString::String(
                "workspace-resource-error".to_owned(),
            ))
            && diagnostic.message.contains("workspace scan worker failed")
    }));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn transient_scan_failure_is_published_and_cleared_after_recovery() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("adocweave-scan-recovery-{unique}"));
    fs::create_dir_all(&root).expect("workspace");
    let config = root.join(adocweave_config::FILE_NAME);
    let document_path = root.join("root.adoc");
    fs::write(&config, "schema-version = 2\n").expect("configuration");
    fs::write(&document_path, "= Root\n").expect("document");
    let root_uri = lsp::Url::from_directory_path(&root).expect("root URI");
    let document_uri = lsp::Url::from_file_path(&document_path).expect("document URI");
    let mut service = Session::default();
    initialize_with_params(
        &mut service,
        typed(json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        })),
    );
    open(&mut service, document_uri.as_str(), 1, "= Root\n");
    let previous = service
        .workspace_resource(&document_uri)
        .expect("workspace resource")
        .clone();

    fs::remove_file(&config).expect("remove configuration");
    fs::create_dir(&config).expect("unreadable configuration entry");
    let failed = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(failed).jobs;

    assert_eq!(
        jobs.len(),
        1,
        "the retained snapshot failure must be published"
    );
    for job in jobs {
        adopt(&mut service, job);
    }
    assert_eq!(
        service
            .workspace_resource(&document_uri)
            .expect("retained workspace resource"),
        previous,
    );
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("failure diagnostics")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code
                == Some(lsp::NumberOrString::String(
                    "workspace-resource-error".to_owned()
                )))
    );

    fs::remove_dir(&config).expect("remove invalid configuration entry");
    fs::write(&config, "schema-version = 2\n").expect("restored configuration");
    let recovered = service.plan_workspace_scan(&adocweave::NeverCancel);
    let jobs = service.apply_workspace_scan(recovered).jobs;
    assert_eq!(jobs.len(), 1, "recovery must clear the published failure");
    for job in jobs {
        adopt(&mut service, job);
    }
    assert!(
        service
            .diagnostics(&document_uri)
            .expect("recovered diagnostics")
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code
                != Some(lsp::NumberOrString::String(
                    "workspace-resource-error".to_owned()
                )))
    );

    fs::remove_dir_all(root).expect("cleanup");
}
