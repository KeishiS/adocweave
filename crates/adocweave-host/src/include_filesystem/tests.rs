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
    LocalFilesystemPolicy::new(
        [root.to_owned()],
        FilesystemReadLimits {
            max_files: 16,
            max_total_bytes,
            max_resource_bytes: 512,
        },
    )
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
