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

fn resources(root: &Path) -> (LocalFilesystemSession, IncludeFilesystem) {
    let policy = LocalFilesystemPolicy::new(
        [root.to_owned()],
        FilesystemReadLimits {
            max_files: 16,
            max_total_bytes: 1_024,
            max_resource_bytes: 512,
        },
    )
    .expect("policy");
    (
        policy.session().expect("filesystem session"),
        IncludeFilesystem::new().expect("include filesystem"),
    )
}

fn source_id(value: &str) -> LogicalSourceId {
    LogicalSourceId::new(value).expect("source ID")
}

fn request(id: &str, base: &Path, target: &str) -> IncludeFilesystemRequest {
    IncludeFilesystemRequest::new(source_id(id), base, target)
}

fn owner(value: &str) -> IncludeFilesystemOwner {
    IncludeFilesystemOwner::new(value).expect("include owner")
}

#[test]
fn found_and_missing_results_expose_only_verified_watch_paths() {
    let root = TestDir::new("outcomes");
    let found = root.path().join("found.adoc");
    fs::write(&found, "included").expect("source");
    let (mut session, mut filesystem) = resources(root.path());
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");

    let IncludeFilesystemOutcome::Found(loaded) = transaction.read(
        owner("document"),
        request("found", root.path(), "found.adoc"),
    ) else {
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

    let IncludeFilesystemOutcome::NotFound(missing) = transaction.read(
        owner("document"),
        request("missing", root.path(), "missing.adoc"),
    ) else {
        panic!("expected missing source");
    };
    assert_eq!(
        missing.watch_candidate().path(),
        root.path().join("missing.adoc")
    );
    let committed = transaction
        .commit(&mut session, &mut filesystem)
        .expect("commit");
    assert_eq!(committed.usage().read_operations, 2);
    assert_eq!(committed.watch_candidates().len(), 2);
}

#[test]
fn dropping_a_transaction_preserves_live_budget_and_bindings() {
    let root = TestDir::new("drop");
    fs::write(root.path().join("source.adoc"), "a").expect("source");
    let (mut session, mut filesystem) = resources(root.path());
    let mut initial = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("initial transaction");
    assert!(matches!(
        initial.read(
            owner("document"),
            request("include", root.path(), "source.adoc")
        ),
        IncludeFilesystemOutcome::Found(_)
    ));
    initial
        .commit(&mut session, &mut filesystem)
        .expect("initial commit");
    assert_eq!(session.budget().bytes(), 1);

    fs::write(root.path().join("source.adoc"), "replacement").expect("replacement");
    let mut discarded = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("discarded transaction");
    assert!(matches!(
        discarded.read(
            owner("document"),
            request("include", root.path(), "source.adoc")
        ),
        IncludeFilesystemOutcome::Found(_)
    ));
    drop(discarded);
    assert_eq!(session.budget().bytes(), 1);
}

#[test]
fn commit_releases_bindings_not_observed_in_the_new_snapshot() {
    let root = TestDir::new("binding-reconciliation");
    fs::write(root.path().join("one.adoc"), "1").expect("one");
    fs::write(root.path().join("two.adoc"), "22").expect("two");
    let (mut session, mut filesystem) = resources(root.path());
    let mut initial = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("initial transaction");
    assert!(matches!(
        initial.read(owner("document"), request("one", root.path(), "one.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    assert!(matches!(
        initial.read(owner("document"), request("two", root.path(), "two.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    initial
        .commit(&mut session, &mut filesystem)
        .expect("initial commit");
    assert_eq!(session.budget().bytes(), 3);

    let mut replacement = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("replacement transaction");
    assert!(matches!(
        replacement.read(owner("document"), request("one", root.path(), "one.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    replacement
        .commit(&mut session, &mut filesystem)
        .expect("replacement commit");
    assert_eq!(session.budget().bytes(), 1);
}

#[test]
fn job_read_limit_is_a_typed_failure_and_cannot_be_committed() {
    let root = TestDir::new("job-limit");
    fs::write(root.path().join("source.adoc"), "three").expect("source");
    let (mut session, mut filesystem) = resources(root.path());
    let limits = FilesystemJobLimits {
        max_read_operations: 1,
        max_read_bytes: 3,
        max_read_probe_bytes: 1,
        max_directory_operations: 1,
        max_directory_entries: 1,
        max_directory_probe_entries: 1,
        max_candidate_changes: 1,
        max_sessions: 1,
    };
    let mut transaction = filesystem
        .transaction(&session, limits)
        .expect("transaction");
    let IncludeFilesystemOutcome::Failed(failed) = transaction.read(
        owner("document"),
        request("include", root.path(), "source.adoc"),
    ) else {
        panic!("expected failed source");
    };
    assert_eq!(
        failed.error(),
        &FilesystemDraftError::Job(FilesystemJobError::Limit(FilesystemJobLimit::ReadBytes {
            limit: 3
        }))
    );
    assert_eq!(
        transaction.commit(&mut session, &mut filesystem),
        Err(FilesystemDraftError::Job(FilesystemJobError::Limit(
            FilesystemJobLimit::ReadBytes { limit: 3 }
        )))
    );
}

#[test]
fn escaping_target_fails_without_an_unverified_watch_candidate() {
    let root = TestDir::new("escape");
    let (mut session, mut filesystem) = resources(root.path());
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");
    let outcome = transaction.read(
        owner("document"),
        request("escape", root.path(), "../outside.adoc"),
    );
    let IncludeFilesystemOutcome::Failed(failed) = outcome else {
        panic!("expected failed source");
    };
    assert!(matches!(
        failed.error(),
        FilesystemDraftError::Resource(ResourceError::OutsideRoots(_))
    ));
    assert!(transaction.commit(&mut session, &mut filesystem).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_cannot_escape_the_retained_root() {
    use std::os::unix::fs::symlink;

    let root = TestDir::new("symlink-root");
    let outside = TestDir::new("symlink-outside");
    fs::write(outside.path().join("secret.adoc"), "secret").expect("outside source");
    symlink(outside.path(), root.path().join("escape")).expect("symlink");
    let (mut session, mut filesystem) = resources(root.path());
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");
    assert!(matches!(
        transaction.read(
            owner("document"),
            request("escape", root.path(), "escape/secret.adoc")
        ),
        IncludeFilesystemOutcome::Failed(_)
    ));
    assert!(transaction.commit(&mut session, &mut filesystem).is_err());
}

#[test]
fn cancellation_prevents_reads_and_atomic_commit() {
    let root = TestDir::new("cancel");
    fs::write(root.path().join("source.adoc"), "source").expect("source");
    let (mut session, mut filesystem) = resources(root.path());
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");
    transaction.cancel().expect("cancel");
    let IncludeFilesystemOutcome::Failed(failed) = transaction.read(
        owner("document"),
        request("include", root.path(), "source.adoc"),
    ) else {
        panic!("expected failed source");
    };
    assert_eq!(
        failed.error(),
        &FilesystemDraftError::Job(FilesystemJobError::Cancelled)
    );
    assert_eq!(
        transaction.commit(&mut session, &mut filesystem),
        Err(FilesystemDraftError::Job(FilesystemJobError::Cancelled))
    );
    assert_eq!(session.budget().bytes(), 0);
}

#[test]
fn inspection_accepts_binary_files_without_charging_read_bytes_or_bindings() {
    let root = TestDir::new("inspect-binary");
    let asset = root.path().join("asset.png");
    fs::write(&asset, [0xff, 0x00, 0x80]).expect("binary asset");
    let (mut session, mut filesystem) = resources(root.path());
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");

    let IncludeFilesystemInspectionOutcome::Found(inspected) =
        transaction.inspect(request("asset", root.path(), "asset.png"))
    else {
        panic!("expected verified asset");
    };
    assert_eq!(inspected.provenance().canonical_path(), asset);
    assert_eq!(
        inspected
            .watch_candidates()
            .iter()
            .map(IncludeWatchCandidate::path)
            .collect::<Vec<_>>(),
        [asset.as_path()]
    );
    let committed = transaction
        .commit(&mut session, &mut filesystem)
        .expect("commit");
    assert_eq!(committed.usage().read_operations, 1);
    assert_eq!(committed.usage().read_bytes, 0);
    assert_eq!(session.budget().bytes(), 0);
    assert!(filesystem.bindings.is_empty());
}

#[test]
fn inspection_reports_missing_and_obeys_the_shared_operation_limit() {
    let root = TestDir::new("inspect-limit");
    let (mut session, mut filesystem) = resources(root.path());
    let limits = FilesystemJobLimits {
        max_read_operations: 1,
        ..FilesystemJobLimits::unbounded()
    };
    let mut transaction = filesystem
        .transaction(&session, limits)
        .expect("transaction");
    let IncludeFilesystemInspectionOutcome::NotFound(missing) =
        transaction.inspect(request("first", root.path(), "first.png"))
    else {
        panic!("expected missing asset");
    };
    assert_eq!(
        missing.watch_candidate().path(),
        root.path().join("first.png")
    );
    let IncludeFilesystemInspectionOutcome::Failed(failed) =
        transaction.inspect(request("second", root.path(), "second.png"))
    else {
        panic!("expected limited inspection");
    };
    assert_eq!(
        failed.error(),
        &FilesystemDraftError::Job(FilesystemJobError::Limit(
            FilesystemJobLimit::ReadOperations { limit: 1 }
        ))
    );
    assert!(transaction.commit(&mut session, &mut filesystem).is_err());
}

#[cfg(unix)]
#[test]
fn inspection_rejects_a_symlink_escape_without_returning_a_watch_path() {
    use std::os::unix::fs::symlink;

    let root = TestDir::new("inspect-symlink-root");
    let outside = TestDir::new("inspect-symlink-outside");
    fs::write(outside.path().join("asset.png"), "outside").expect("outside asset");
    symlink(outside.path(), root.path().join("escape")).expect("symlink");
    let (mut session, mut filesystem) = resources(root.path());
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");
    assert!(matches!(
        transaction.inspect(request("asset", root.path(), "escape/asset.png")),
        IncludeFilesystemInspectionOutcome::Failed(_)
    ));
    assert!(transaction.commit(&mut session, &mut filesystem).is_err());
}

#[test]
fn replacing_one_owner_does_not_release_another_owners_bindings() {
    let root = TestDir::new("owner-isolation");
    fs::write(root.path().join("one.adoc"), "1").expect("one");
    fs::write(root.path().join("two.adoc"), "22").expect("two");
    let (mut session, mut filesystem) = resources(root.path());
    let first_owner = owner("first-document");
    let second_owner = owner("second-document");
    let mut initial = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("initial transaction");
    assert!(matches!(
        initial.read(first_owner.clone(), request("one", root.path(), "one.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    assert!(matches!(
        initial.read(
            second_owner.clone(),
            request("two", root.path(), "two.adoc")
        ),
        IncludeFilesystemOutcome::Found(_)
    ));
    initial
        .commit(&mut session, &mut filesystem)
        .expect("initial commit");

    let mut replacement = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("replacement transaction");
    assert!(matches!(
        replacement.read(first_owner.clone(), request("one", root.path(), "one.adoc")),
        IncludeFilesystemOutcome::Found(_)
    ));
    replacement
        .commit(&mut session, &mut filesystem)
        .expect("replacement commit");

    assert_eq!(session.budget().bytes(), 3);
    assert_eq!(filesystem.bindings[&first_owner].resources.len(), 1);
    assert_eq!(filesystem.bindings[&second_owner].resources.len(), 1);
}

#[test]
fn transaction_rejects_a_different_registry_at_commit() {
    let root = TestDir::new("foreign-registry");
    fs::write(root.path().join("source.adoc"), "source").expect("source");
    let (mut session, filesystem) = resources(root.path());
    let mut foreign = IncludeFilesystem::new().expect("foreign registry");
    let mut transaction = filesystem
        .transaction(&session, FilesystemJobLimits::unbounded())
        .expect("transaction");
    assert!(matches!(
        transaction.read(
            owner("document"),
            request("include", root.path(), "source.adoc")
        ),
        IncludeFilesystemOutcome::Found(_)
    ));
    assert_eq!(
        transaction.commit(&mut session, &mut foreign),
        Err(FilesystemDraftError::InvalidDraft)
    );
    assert_eq!(session.budget().bytes(), 0);
}
