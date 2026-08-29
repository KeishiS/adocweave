use super::*;
use crate::{FilesystemJobLimit, FilesystemReadLimits, LocalFilesystemPolicy};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "adocweave-host-include-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test directory");
        Self(path.canonicalize().expect("canonical test directory"))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn session(root: &Path, max_total_bytes: u64) -> LocalFilesystemSession {
    session_with_limits(
        root,
        FilesystemReadLimits {
            max_files: 16,
            max_total_bytes,
            max_resource_bytes: 512,
        },
    )
}

fn session_with_limits(root: &Path, limits: FilesystemReadLimits) -> LocalFilesystemSession {
    LocalFilesystemPolicy::new([root.to_owned()], limits)
        .expect("policy")
        .session()
        .expect("filesystem session")
}

fn source_id(value: &str) -> LogicalSourceId {
    LogicalSourceId::new(value).expect("source ID")
}

fn request(id: &str, base: &Path, target: &str) -> IncludeFilesystemRequest {
    IncludeFilesystemRequest::new(source_id(id), base, target)
}

fn path_request(id: &str, path: &Path) -> IncludeFilesystemPathRequest {
    IncludeFilesystemPathRequest::new(source_id(id), path)
}

#[test]
fn transaction_returns_content_provenance_binding_and_safe_watch_paths() {
    let root = TestDir::new("outcome");
    let found = root.path().join("found.adoc");
    fs::write(&found, "included").expect("source");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");
    let IncludeFilesystemOutcome::Found(loaded) =
        transaction.read(request("found", root.path(), "found.adoc"))
    else {
        panic!("expected found source");
    };
    assert_eq!(loaded.source(), "included");
    assert_eq!(loaded.provenance().canonical_path(), found);
    assert_eq!(
        loaded
            .watch_candidates()
            .iter()
            .map(IncludeWatchCandidate::path)
            .collect::<Vec<_>>(),
        [found.as_path()]
    );
    transaction.commit(&mut session).expect("commit");
    assert_eq!(job.finish().expect("finish").read_operations, 1);
}

#[test]
fn dropping_a_transaction_preserves_live_state() {
    let root = TestDir::new("drop");
    fs::write(root.path().join("source.adoc"), "old").expect("source");
    let mut session = session(root.path(), 1_024);
    let first_job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut first = first_job.transaction(&session).expect("transaction");
    assert!(matches!(
        first.read(request("source", root.path(), "source.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    first.commit(&mut session).expect("commit");
    first_job.finish().expect("finish");
    assert_eq!(session.budget().bytes(), 3);

    fs::write(root.path().join("source.adoc"), "replacement").expect("replacement");
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut discarded = job.transaction(&session).expect("transaction");
    assert!(matches!(
        discarded.read(request("source", root.path(), "source.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    drop(discarded);
    job.finish().expect("finish");
    assert_eq!(session.budget().bytes(), 3);
}

#[test]
fn explicit_release_is_generation_safe_when_a_binding_is_stale() {
    let root = TestDir::new("stale-release");
    let path = root.path().join("source.adoc");
    fs::write(&path, "old").expect("source");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut first = job.transaction(&session).expect("transaction");
    let IncludeFilesystemOutcome::Found(source) =
        first.read(request("source", root.path(), "source.adoc"))
    else {
        panic!("expected source");
    };
    let stale = source.binding().clone();
    first.commit(&mut session).expect("commit");
    job.finish().expect("finish");

    fs::write(&path, "new value").expect("replacement");
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut replacement = job.transaction(&session).expect("transaction");
    let IncludeFilesystemOutcome::Found(source) =
        replacement.read(request("source", root.path(), "source.adoc"))
    else {
        panic!("expected replacement");
    };
    let current = source.binding().clone();
    replacement.commit(&mut session).expect("commit");
    job.finish().expect("finish");

    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut release = job.transaction(&session).expect("transaction");
    release.release(&stale).expect("stale release is harmless");
    release.commit(&mut session).expect("commit");
    job.finish().expect("finish");
    assert_eq!(session.budget().bytes(), 9);

    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut release = job.transaction(&session).expect("transaction");
    release.release(&current).expect("current release");
    release.commit(&mut session).expect("commit");
    job.finish().expect("finish");
    assert_eq!(session.budget().bytes(), 0);
}

#[test]
fn failed_atomic_operation_is_terminal() {
    let root = TestDir::new("terminal-failure");
    fs::write(root.path().join("invalid.adoc"), [0xff]).expect("invalid");
    fs::write(root.path().join("valid.adoc"), "valid").expect("valid");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");
    assert!(matches!(
        transaction.read(request("invalid", root.path(), "invalid.adoc")),
        IncludeFilesystemOutcome::Failed(_)
    ));
    let IncludeFilesystemOutcome::Failed(failed) =
        transaction.read(request("valid", root.path(), "valid.adoc"))
    else {
        panic!("poisoned transaction must fail");
    };
    assert_eq!(failed.error(), &FilesystemDraftError::PoisonedDraft);
    assert_eq!(
        transaction.commit(&mut session),
        Err(FilesystemDraftError::PoisonedDraft)
    );
    assert_eq!(session.budget().bytes(), 0);
}

#[test]
fn live_failure_does_not_poison_later_relative_or_absolute_reads() {
    let root = TestDir::new("live-lenient");
    let invalid = root.path().join("invalid.adoc");
    fs::write(&invalid, [0xff]).expect("invalid");
    fs::write(root.path().join("valid.adoc"), "valid").expect("valid");
    let mut session = session(root.path(), 1_024);
    let filesystem = IncludeFilesystem::new();
    assert!(matches!(
        filesystem.read_utf8(&mut session, path_request("invalid", &invalid)),
        IncludeFilesystemOutcome::Failed(_)
    ));
    assert!(matches!(
        filesystem.read(&mut session, request("valid", root.path(), "valid.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
}

#[test]
fn absolute_read_reports_budget_exhaustion_without_poisoning() {
    let root = TestDir::new("absolute-budget");
    let small = root.path().join("small.adoc");
    let large = root.path().join("large.adoc");
    fs::write(&small, "ok").expect("small source");
    fs::write(&large, "four").expect("large source");
    let mut session = session(root.path(), 3);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");
    assert!(matches!(
        transaction
            .read_utf8_within_budget(path_request("missing", &root.path().join("missing.adoc"))),
        IncludeFilesystemBudgetedOutcome::NotFound(_)
    ));
    assert!(matches!(
        transaction.read_utf8_within_budget(path_request("small", &small)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    assert!(matches!(
        transaction.read_utf8_within_budget(path_request("large", &large)),
        IncludeFilesystemBudgetedOutcome::BudgetExhausted { .. }
    ));
    transaction.commit(&mut session).expect("commit");
    assert_eq!(job.finish().expect("finish").read_operations, 3);
    assert_eq!(session.budget().bytes(), 2);
}

#[test]
fn additional_read_limits_bound_io_without_replacing_the_session_budget() {
    let root = TestDir::new("additional-read-limits");
    let retained = root.path().join("retained.adoc");
    let exact = root.path().join("exact.adoc");
    let large = root.path().join("large.adoc");
    let later = root.path().join("later.adoc");
    fs::write(&retained, "retained").expect("retained source");
    fs::write(&exact, "four").expect("exact source");
    fs::write(&large, "x".repeat(64 * 1024)).expect("large source");
    fs::write(&later, "yes").expect("later source");
    let mut session = session(root.path(), 1_024 * 1024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");

    let mut first = job.transaction(&session).expect("first transaction");
    assert!(matches!(
        first.read_utf8_within_budget(path_request("retained", &retained)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    first.commit(&mut session).expect("commit retained source");

    let limits = FilesystemReadLimits {
        max_files: 16,
        max_total_bytes: 4,
        max_resource_bytes: 4,
    };
    let mut exact_read = job.transaction(&session).expect("exact transaction");
    assert!(matches!(
        exact_read.read_utf8_within_limits(path_request("exact", &exact), limits),
        IncludeFilesystemLimitedOutcome::Found(_)
    ));
    exact_read
        .commit(&mut session)
        .expect("commit exact source");

    let before = job
        .usage()
        .expect("usage before bounded failure")
        .read_bytes;
    let mut bounded = job.transaction(&session).expect("bounded transaction");
    assert!(matches!(
        bounded.read_utf8_within_limits(path_request("large", &large), limits),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Additional,
            ..
        }
    ));
    let after_limit = job.usage().expect("usage after bounded failure").read_bytes;
    assert_eq!(after_limit - before, 5);
    assert!(matches!(
        bounded.read_utf8_within_budget(path_request("later", &later)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    bounded
        .commit(&mut session)
        .expect("limit leaves draft usable");
    let usage = job.finish().expect("finish");
    assert_eq!(usage.read_bytes - after_limit, 3);
    assert_eq!(session.budget().bytes(), 15);
}

#[test]
fn additional_resource_limit_wins_over_a_completed_request_operation() {
    let root = TestDir::new("additional-versus-operation");
    let first_path = root.path().join("first.adoc");
    let large = root.path().join("large.adoc");
    fs::write(&first_path, "ok").expect("first source");
    fs::write(&large, "x".repeat(64 * 1024)).expect("large source");
    let mut session = session(root.path(), 1_024);
    let mut job_limits = FilesystemJobLimits::unbounded();
    job_limits.max_read_operations = 2;
    let job = IncludeFilesystemJob::new(job_limits).expect("job");

    let mut first = job.transaction(&session).expect("first transaction");
    assert!(matches!(
        first.read_utf8_within_budget(path_request("first", &first_path)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    first.commit(&mut session).expect("commit first source");

    let mut limited = job.transaction(&session).expect("limited transaction");
    assert!(matches!(
        limited.read_utf8_within_limits(
            path_request("large", &large),
            FilesystemReadLimits {
                max_files: 16,
                max_total_bytes: 1_024,
                max_resource_bytes: 4,
            },
        ),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Additional,
            ..
        }
    ));
    limited
        .commit(&mut session)
        .expect("additional limit keeps transaction usable");
    let usage = job.finish().expect("finish");
    assert_eq!(usage.read_operations, 2);
    assert_eq!(usage.read_bytes, 7);
}

#[test]
fn additional_limit_does_not_fix_a_failure_inside_the_transaction() {
    let root = TestDir::new("additional-retry-same-transaction");
    let path = root.path().join("source.adoc");
    fs::write(&path, "eight888").expect("source");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");

    assert!(matches!(
        transaction.read_utf8_within_limits(
            path_request("source", &path),
            FilesystemReadLimits {
                max_files: 16,
                max_total_bytes: 4,
                max_resource_bytes: 4,
            },
        ),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Additional,
            ..
        }
    ));
    assert!(matches!(
        transaction.read_utf8_within_budget(path_request("source", &path)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    transaction
        .commit(&mut session)
        .expect("wide retry commits");
    let usage = job.finish().expect("finish");
    assert_eq!(usage.read_operations, 2);
    assert_eq!(usage.read_bytes, 13);
    assert_eq!(session.budget().bytes(), 8);
}

#[test]
fn additional_limit_does_not_commit_a_failure_with_an_unrelated_read() {
    let root = TestDir::new("additional-retry-next-transaction");
    let path = root.path().join("source.adoc");
    let other = root.path().join("other.adoc");
    fs::write(&path, "eight888").expect("source");
    fs::write(&other, "ok").expect("other source");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut first = job.transaction(&session).expect("first transaction");

    assert!(matches!(
        first.read_utf8_within_limits(
            path_request("source", &path),
            FilesystemReadLimits {
                max_files: 16,
                max_total_bytes: 4,
                max_resource_bytes: 4,
            },
        ),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Additional,
            ..
        }
    ));
    assert!(matches!(
        first.read_utf8_within_budget(path_request("other", &other)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    first.commit(&mut session).expect("commit unrelated source");

    let mut second = job.transaction(&session).expect("second transaction");
    assert!(matches!(
        second.read_utf8_within_budget(path_request("source", &path)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    second.commit(&mut session).expect("commit wide retry");
    let usage = job.finish().expect("finish");
    assert_eq!(usage.read_operations, 3);
    assert_eq!(usage.read_bytes, 15);
    assert_eq!(session.budget().bytes(), 10);
}

#[test]
fn simultaneous_request_file_limit_is_reported_as_established() {
    let root = TestDir::new("established-file-limit");
    let path = root.path().join("source.adoc");
    fs::write(&path, "text").expect("source");
    let session = session(root.path(), 1_024);
    let mut job_limits = FilesystemJobLimits::unbounded();
    job_limits.max_read_operations = 0;
    let job = IncludeFilesystemJob::new(job_limits).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");

    assert!(matches!(
        transaction.read_utf8_within_limits(
            path_request("source", &path),
            FilesystemReadLimits {
                max_files: 0,
                max_total_bytes: 1_024,
                max_resource_bytes: 512,
            },
        ),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Established(FilesystemDraftError::Job(
                FilesystemJobError::Limit(FilesystemJobLimit::ReadOperations { limit: 0 })
            )),
            ..
        }
    ));
}

#[test]
fn simultaneous_request_total_limit_is_reported_as_established() {
    let root = TestDir::new("established-total-limit");
    let path = root.path().join("source.adoc");
    fs::write(&path, "x".repeat(64 * 1024)).expect("source");
    let session = session(root.path(), 1_024);
    let mut job_limits = FilesystemJobLimits::unbounded();
    job_limits.max_read_bytes = 4;
    job_limits.max_read_probe_bytes = 1;
    let job = IncludeFilesystemJob::new(job_limits).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");

    assert!(matches!(
        transaction.read_utf8_within_limits(
            path_request("source", &path),
            FilesystemReadLimits {
                max_files: 16,
                max_total_bytes: 4,
                max_resource_bytes: 512,
            },
        ),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Established(FilesystemDraftError::Job(
                FilesystemJobError::Limit(
                    FilesystemJobLimit::ReadBytes { limit: 4 }
                        | FilesystemJobLimit::ReadProbeBytes { limit: 1 }
                )
            )),
            ..
        }
    ));
}

#[test]
fn simultaneous_request_resource_limit_is_reported_as_established() {
    let root = TestDir::new("established-resource-limit");
    let path = root.path().join("source.adoc");
    fs::write(&path, "x".repeat(64 * 1024)).expect("source");
    let mut session = session_with_limits(
        root.path(),
        FilesystemReadLimits {
            max_files: 16,
            max_total_bytes: 1_024,
            max_resource_bytes: 4,
        },
    );
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");

    assert!(matches!(
        transaction.read_utf8_within_limits(
            path_request("source", &path),
            FilesystemReadLimits {
                max_files: 16,
                max_total_bytes: 1_024,
                max_resource_bytes: 4,
            },
        ),
        IncludeFilesystemLimitedOutcome::Limit {
            cause: IncludeFilesystemReadLimit::Established(FilesystemDraftError::Resource(
                ResourceError::ResourceTooLarge(_)
            )),
            ..
        }
    ));
    transaction
        .commit(&mut session)
        .expect("resource limit keeps transaction usable");
    assert_eq!(job.finish().expect("finish").read_bytes, 5);
}

#[test]
fn inspection_reads_no_bytes_and_creates_no_binding() {
    let root = TestDir::new("inspect");
    let asset = root.path().join("asset.png");
    fs::write(&asset, [0xff, 0x00, 0x80]).expect("asset");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut transaction = job.transaction(&session).expect("transaction");
    assert!(matches!(
        transaction.inspect(request("asset", root.path(), "asset.png")),
        IncludeFilesystemInspectionOutcome::Found(_)
    ));
    transaction.commit(&mut session).expect("commit");
    let usage = job.finish().expect("finish");
    assert_eq!(usage.read_bytes, 0);
    assert_eq!(session.budget().bytes(), 0);
}

#[test]
fn inspection_within_uses_a_wider_explicit_authority_for_parent_targets() {
    let project = TestDir::new("inspect-authority");
    let docs = project.path().join("docs");
    fs::create_dir(&docs).expect("docs");
    let asset = project.path().join("asset.png");
    fs::write(&asset, "asset").expect("asset");
    let mut session = LocalFilesystemPolicy::new(
        [project.path().to_owned(), docs.clone()],
        FilesystemReadLimits::default(),
    )
    .expect("policy")
    .session()
    .expect("session");

    let IncludeFilesystemInspectionOutcome::Found(found) = IncludeFilesystem::new().inspect_within(
        &mut session,
        project.path(),
        request("asset", &docs, "../asset.png"),
    ) else {
        panic!("the project authority should contain the parent target");
    };
    assert_eq!(found.provenance().canonical_path(), asset);
}

#[test]
fn inspection_within_rejects_targets_outside_the_explicit_authority() {
    let parent = TestDir::new("inspect-outside-parent");
    let project = parent.path().join("project");
    let docs = project.join("docs");
    fs::create_dir_all(&docs).expect("docs");
    fs::write(parent.path().join("secret.png"), "secret").expect("outside asset");
    let mut session = LocalFilesystemPolicy::new(
        [project.clone(), docs.clone()],
        FilesystemReadLimits::default(),
    )
    .expect("policy")
    .session()
    .expect("session");

    assert!(matches!(
        IncludeFilesystem::new().inspect_within(
            &mut session,
            &project,
            request("secret", &docs, "../../secret.png"),
        ),
        IncludeFilesystemInspectionOutcome::Failed(_)
    ));
}

#[cfg(unix)]
#[test]
fn inspection_within_does_not_retry_a_symlink_escape_under_a_wider_root() {
    use std::os::unix::fs::symlink;

    let project = TestDir::new("inspect-no-symlink-fallback");
    let docs = project.path().join("docs");
    fs::create_dir(&docs).expect("docs");
    fs::write(project.path().join("asset.png"), "asset").expect("asset");
    symlink("../asset.png", docs.join("asset-link.png")).expect("symlink");
    let mut session = LocalFilesystemPolicy::new(
        [project.path().to_owned(), docs.clone()],
        FilesystemReadLimits::default(),
    )
    .expect("policy")
    .session()
    .expect("session");

    assert!(matches!(
        IncludeFilesystem::new().inspect_within(
            &mut session,
            project.path(),
            request("asset", &docs, "asset-link.png"),
        ),
        IncludeFilesystemInspectionOutcome::Failed(_)
    ));
}

#[test]
fn inspection_within_shares_the_live_session_path_budget() {
    let project = TestDir::new("inspect-shared-budget");
    let docs = project.path().join("docs");
    fs::create_dir(&docs).expect("docs");
    fs::write(docs.join("root.adoc"), "root").expect("root source");
    fs::write(project.path().join("asset.png"), "asset").expect("asset");
    let mut session = LocalFilesystemPolicy::new(
        [project.path().to_owned(), docs.clone()],
        FilesystemReadLimits {
            max_files: 1,
            ..FilesystemReadLimits::default()
        },
    )
    .expect("policy")
    .session()
    .expect("session");
    let filesystem = IncludeFilesystem::new();
    assert!(matches!(
        filesystem.read_utf8(&mut session, path_request("root", &docs.join("root.adoc")),),
        IncludeFilesystemOutcome::Found(_)
    ));

    assert!(matches!(
        filesystem.inspect_within(
            &mut session,
            project.path(),
            request("asset", &docs, "../asset.png"),
        ),
        IncludeFilesystemInspectionOutcome::Failed(_)
    ));
}

#[test]
fn one_job_enforces_a_shared_limit_across_sessions() {
    let first_root = TestDir::new("shared-first");
    let second_root = TestDir::new("shared-second");
    fs::write(first_root.path().join("one.adoc"), "1").expect("first");
    fs::write(second_root.path().join("two.adoc"), "2").expect("second");
    let mut first_session = session(first_root.path(), 1_024);
    let mut second_session = session(second_root.path(), 1_024);
    let limits = FilesystemJobLimits {
        max_read_operations: 1,
        max_sessions: 2,
        ..FilesystemJobLimits::unbounded()
    };
    let job = IncludeFilesystemJob::new(limits).expect("job");
    let mut first = job.transaction(&first_session).expect("first transaction");
    assert!(matches!(
        first.read(request("one", first_root.path(), "one.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    first.commit(&mut first_session).expect("commit");
    let mut second = job
        .transaction(&second_session)
        .expect("second transaction");
    let IncludeFilesystemOutcome::Failed(failed) =
        second.read(request("two", second_root.path(), "two.adoc"))
    else {
        panic!("shared limit must reject second read");
    };
    assert_eq!(
        failed.error(),
        &FilesystemDraftError::Job(FilesystemJobError::Limit(
            FilesystemJobLimit::ReadOperations { limit: 1 }
        ))
    );
    assert!(second.commit(&mut second_session).is_err());
}

#[test]
fn live_mutation_makes_an_atomic_transaction_stale() {
    let root = TestDir::new("stale-transaction");
    let path = root.path().join("source.adoc");
    fs::write(&path, "source").expect("source");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let transaction = job.transaction(&session).expect("transaction");
    assert!(matches!(
        IncludeFilesystem::new().read_utf8(&mut session, path_request("live", &path)),
        IncludeFilesystemOutcome::Found(_)
    ));
    assert_eq!(
        transaction.commit(&mut session),
        Err(FilesystemDraftError::InvalidDraft)
    );
}

#[test]
fn superseding_transaction_rejects_old_work_and_commits_only_its_candidate() {
    let root = TestDir::new("superseding-transaction");
    let old_path = root.path().join("old.adoc");
    let new_path = root.path().join("new.adoc");
    fs::write(&old_path, "old").expect("old source");
    fs::write(&new_path, "replacement").expect("new source");
    let mut session = session(root.path(), 1_024);
    let job = IncludeFilesystemJob::new(FilesystemJobLimits::unbounded()).expect("job");
    let mut old = job.transaction(&session).expect("old transaction");
    assert!(matches!(
        old.read_utf8_within_budget(path_request("old", &old_path)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));

    let mut replacement = job
        .superseding_transaction(&mut session)
        .expect("replacement transaction");
    assert_eq!(
        old.validate(&session),
        Err(FilesystemDraftError::InvalidDraft)
    );
    let IncludeFilesystemBudgetedOutcome::Failed(failed) =
        old.read_utf8_within_budget(path_request("old-again", &old_path))
    else {
        panic!("superseded transaction must reject later work");
    };
    assert_eq!(failed.error(), &FilesystemDraftError::InvalidDraft);
    assert_eq!(
        old.commit(&mut session),
        Err(FilesystemDraftError::InvalidDraft)
    );
    assert!(matches!(
        replacement.read_utf8_within_budget(path_request("new", &new_path)),
        IncludeFilesystemBudgetedOutcome::Found(_)
    ));
    replacement
        .commit(&mut session)
        .expect("commit replacement");
    job.finish().expect("finish job");

    assert_eq!(session.budget().bytes(), "replacement".len() as u64);
}

#[cfg(unix)]
#[test]
fn live_read_and_inspection_reject_symlink_escape() {
    use std::os::unix::fs::symlink;
    let root = TestDir::new("symlink-root");
    let outside = TestDir::new("symlink-outside");
    fs::write(outside.path().join("secret.adoc"), "secret").expect("outside");
    symlink(outside.path(), root.path().join("escape")).expect("symlink");
    let mut session = session(root.path(), 1_024);
    let filesystem = IncludeFilesystem::new();
    fs::write(root.path().join("safe.png"), "safe").expect("safe asset");
    let escape_request = || request("escape", root.path(), "escape/secret.adoc");
    assert!(matches!(
        filesystem.read(&mut session, escape_request()),
        IncludeFilesystemOutcome::Failed(_)
    ));
    assert!(matches!(
        filesystem.inspect(&mut session, escape_request()),
        IncludeFilesystemInspectionOutcome::Failed(_)
    ));
    assert!(matches!(
        filesystem.inspect(&mut session, request("safe", root.path(), "safe.png")),
        IncludeFilesystemInspectionOutcome::Found(_)
    ));
}
