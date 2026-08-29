//! High-level filesystem boundary for native include drivers.
//!
//! Atomic transactions own the draft and expose opaque bindings. Consumers
//! retain those bindings in their existing state and release obsolete values
//! explicitly. Lenient live operations are separate because one failed
//! optional target must not poison later CLI validation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::local_resource::{FilesystemInspectOutcome, FilesystemLimitedReadOutcome};
use crate::{
    FilesystemDraftError, FilesystemJobCoordinator, FilesystemJobError, FilesystemJobLimits,
    FilesystemJobUsage, FilesystemReadLimits, FilesystemReadOutcome, FilesystemResourceBinding,
    LocalFilesystemDraft, LocalFilesystemSession, LogicalSourceId, ResourceError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemRequest {
    source_id: LogicalSourceId,
    base: PathBuf,
    target: Arc<str>,
}

impl IncludeFilesystemRequest {
    pub fn new(
        source_id: LogicalSourceId,
        base: impl Into<PathBuf>,
        target: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            source_id,
            base: base.into(),
            target: target.into(),
        }
    }

    pub fn source_id(&self) -> &LogicalSourceId {
        &self.source_id
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemPathRequest {
    source_id: LogicalSourceId,
    path: PathBuf,
}

impl IncludeFilesystemPathRequest {
    pub fn new(source_id: LogicalSourceId, path: impl Into<PathBuf>) -> Self {
        Self {
            source_id,
            path: path.into(),
        }
    }

    pub fn source_id(&self) -> &LogicalSourceId {
        &self.source_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Generation-specific ownership returned to the consumer.
///
/// This value is intentionally opaque and can only be passed to `release`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemBinding(FilesystemResourceBinding);

impl From<FilesystemResourceBinding> for IncludeFilesystemBinding {
    /// Wraps a claim returned by a lower-level scan without exposing it again.
    fn from(binding: FilesystemResourceBinding) -> Self {
        Self(binding)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IncludeWatchCandidate(PathBuf);

impl IncludeWatchCandidate {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemProvenance {
    canonical_path: PathBuf,
}

impl IncludeFilesystemProvenance {
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemSource {
    source_id: LogicalSourceId,
    source: Arc<str>,
    provenance: IncludeFilesystemProvenance,
    binding: IncludeFilesystemBinding,
    watch_candidates: Vec<IncludeWatchCandidate>,
}

impl IncludeFilesystemSource {
    pub fn source_id(&self) -> &LogicalSourceId {
        &self.source_id
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn provenance(&self) -> &IncludeFilesystemProvenance {
        &self.provenance
    }

    pub fn binding(&self) -> &IncludeFilesystemBinding {
        &self.binding
    }

    pub fn watch_candidates(&self) -> &[IncludeWatchCandidate] {
        &self.watch_candidates
    }

    pub fn into_parts(self) -> (LogicalSourceId, Arc<str>, IncludeFilesystemBinding) {
        (self.source_id, self.source, self.binding)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemInspection {
    source_id: LogicalSourceId,
    provenance: IncludeFilesystemProvenance,
    watch_candidates: Vec<IncludeWatchCandidate>,
}

impl IncludeFilesystemInspection {
    pub fn source_id(&self) -> &LogicalSourceId {
        &self.source_id
    }

    pub fn provenance(&self) -> &IncludeFilesystemProvenance {
        &self.provenance
    }

    pub fn watch_candidates(&self) -> &[IncludeWatchCandidate] {
        &self.watch_candidates
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MissingIncludeFilesystemSource {
    source_id: LogicalSourceId,
    watch_candidate: IncludeWatchCandidate,
}

impl MissingIncludeFilesystemSource {
    pub fn source_id(&self) -> &LogicalSourceId {
        &self.source_id
    }

    pub fn watch_candidate(&self) -> &IncludeWatchCandidate {
        &self.watch_candidate
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedIncludeFilesystemSource {
    source_id: LogicalSourceId,
    error: FilesystemDraftError,
}

impl FailedIncludeFilesystemSource {
    pub fn source_id(&self) -> &LogicalSourceId {
        &self.source_id
    }

    pub fn error(&self) -> &FilesystemDraftError {
        &self.error
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemOutcome {
    Found(IncludeFilesystemSource),
    NotFound(MissingIncludeFilesystemSource),
    Failed(FailedIncludeFilesystemSource),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemBudgetedOutcome {
    Found(IncludeFilesystemSource),
    NotFound(MissingIncludeFilesystemSource),
    BudgetExhausted { source_id: LogicalSourceId },
    Failed(FailedIncludeFilesystemSource),
}

/// The limit which refused a read with an additional operation ceiling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemReadLimit {
    /// The additional ceiling supplied for this operation was narrower.
    Additional,
    /// A pre-existing session or shared-job ceiling was narrower.
    Established(FilesystemDraftError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemLimitedOutcome {
    Found(IncludeFilesystemSource),
    NotFound(MissingIncludeFilesystemSource),
    Limit {
        source_id: LogicalSourceId,
        cause: IncludeFilesystemReadLimit,
    },
    Failed(FailedIncludeFilesystemSource),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemInspectionOutcome {
    Found(IncludeFilesystemInspection),
    NotFound(MissingIncludeFilesystemSource),
    Failed(FailedIncludeFilesystemSource),
}

/// Stateless entry point for lenient operations on live session state.
///
/// A live operation does not poison later live operations. It does invalidate
/// the commit revision of an outstanding atomic transaction for the same
/// session, but it does not release that transaction's draft lease. The stale
/// transaction may continue isolated work until it is dropped, and another
/// atomic transaction cannot start in the meantime. Use
/// [`IncludeFilesystemJob::superseding_transaction`] when a replacement must
/// acquire the lease immediately.
#[derive(Clone, Copy, Debug, Default)]
pub struct IncludeFilesystem;

impl IncludeFilesystem {
    pub const fn new() -> Self {
        Self
    }

    pub fn read(
        &self,
        session: &mut LocalFilesystemSession,
        request: IncludeFilesystemRequest,
    ) -> IncludeFilesystemOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        let outcome = session.read_target_utf8_outcome(source_id.clone(), &base, &target);
        map_live_read(source_id, outcome)
    }

    pub fn read_utf8(
        &self,
        session: &mut LocalFilesystemSession,
        request: IncludeFilesystemPathRequest,
    ) -> IncludeFilesystemOutcome {
        let IncludeFilesystemPathRequest { source_id, path } = request;
        let outcome = session.read_utf8_outcome(source_id.clone(), &path);
        map_live_read(source_id, outcome)
    }

    pub fn inspect(
        &self,
        session: &mut LocalFilesystemSession,
        request: IncludeFilesystemRequest,
    ) -> IncludeFilesystemInspectionOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        map_inspection(
            source_id.clone(),
            session
                .inspect_target_outcome(source_id, &base, &target)
                .map_err(FilesystemDraftError::from),
        )
    }

    /// Inspects a target within one explicit authority while retaining the
    /// session's shared cache and budget.
    ///
    /// A session may contain both the authority root and narrower roots for
    /// individual documents. The host chooses the deepest configured root
    /// which contains both the base and the lexically normalized target. It
    /// then performs exactly one handle-relative inspection. A filesystem
    /// error, including a symbolic-link escape, is never retried under a wider
    /// root.
    pub fn inspect_within(
        &self,
        session: &mut LocalFilesystemSession,
        authority: &Path,
        request: IncludeFilesystemRequest,
    ) -> IncludeFilesystemInspectionOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        map_inspection(
            source_id.clone(),
            session
                .inspect_target_within_outcome(source_id, authority, &base, &target)
                .map_err(FilesystemDraftError::from),
        )
    }
}

/// One job budget shared by transactions from any authorized session.
#[derive(Debug)]
pub struct IncludeFilesystemJob {
    coordinator: FilesystemJobCoordinator,
}

impl IncludeFilesystemJob {
    pub fn new(limits: FilesystemJobLimits) -> Result<Self, FilesystemJobError> {
        Ok(Self {
            coordinator: FilesystemJobCoordinator::new(limits)?,
        })
    }

    /// Creates an owned transaction without retaining the session borrow.
    pub fn transaction(
        &self,
        session: &LocalFilesystemSession,
    ) -> Result<IncludeFilesystemTransaction, FilesystemDraftError> {
        Ok(IncludeFilesystemTransaction {
            draft: Some(session.draft(&self.coordinator)?),
            coordinator: self.coordinator.clone(),
        })
    }

    /// Invalidates an older detached candidate and starts its replacement.
    ///
    /// Native watch updates use this when they must remain atomic while an
    /// analysis still owns an unpublished transaction for the same session.
    /// The superseded transaction can neither perform more I/O nor commit.
    ///
    /// An error does not guarantee that the older transaction remains valid.
    /// Invalidating its lease precedes creation of the replacement, and a
    /// failure in that second step does not restore the old lease.
    pub fn superseding_transaction(
        &self,
        session: &mut LocalFilesystemSession,
    ) -> Result<IncludeFilesystemTransaction, FilesystemDraftError> {
        session.supersede_active_draft()?;
        self.transaction(session)
    }

    pub fn usage(&self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        self.coordinator.usage()
    }

    pub fn cancel(&self) -> Result<(), FilesystemJobError> {
        self.coordinator.cancel()
    }

    /// Finishes the job after all transactions have been committed or dropped.
    ///
    /// Transaction commits never finish the shared job. Conversely, this
    /// method changes no session state and cannot roll back earlier commits.
    pub fn finish(self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        self.coordinator.finish_with_usage()
    }
}

/// An isolated atomic filesystem candidate.
///
/// Any `Failed` operation is terminal: the draft is poisoned and all later
/// operations and commit are rejected. Use live methods on
/// [`IncludeFilesystem`] when optional failures must not stop later work.
#[must_use = "an include filesystem transaction must be committed or dropped"]
pub struct IncludeFilesystemTransaction {
    draft: Option<LocalFilesystemDraft>,
    coordinator: FilesystemJobCoordinator,
}

impl IncludeFilesystemTransaction {
    /// Lists safely discovered AsciiDoc paths and reports whether the bounded
    /// walk reached its end.
    pub fn discover_adoc_paths_within_budget(
        &self,
        exclude_directory: impl FnMut(&Path, &Path) -> bool,
    ) -> Result<(Vec<PathBuf>, bool), FilesystemDraftError> {
        self.draft_ref()
            .discover_adoc_paths_within_budget(exclude_directory, || false)
    }

    pub fn read(&mut self, request: IncludeFilesystemRequest) -> IncludeFilesystemOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        let outcome = self
            .draft_mut()
            .read_target_utf8_outcome(source_id.clone(), &base, &target);
        map_draft_read(source_id, outcome)
    }

    pub fn read_utf8_within_budget(
        &mut self,
        request: IncludeFilesystemPathRequest,
    ) -> IncludeFilesystemBudgetedOutcome {
        let IncludeFilesystemPathRequest { source_id, path } = request;
        match self
            .draft_mut()
            .read_utf8_within_budget(source_id.clone(), &path)
        {
            Ok(Some(outcome)) => match map_read(outcome) {
                IncludeFilesystemOutcome::Found(found) => {
                    IncludeFilesystemBudgetedOutcome::Found(found)
                }
                IncludeFilesystemOutcome::NotFound(missing) => {
                    IncludeFilesystemBudgetedOutcome::NotFound(missing)
                }
                IncludeFilesystemOutcome::Failed(_) => unreachable!("successful read mapping"),
            },
            Ok(None) => IncludeFilesystemBudgetedOutcome::BudgetExhausted { source_id },
            Err(error) => IncludeFilesystemBudgetedOutcome::Failed(FailedIncludeFilesystemSource {
                source_id,
                error,
            }),
        }
    }

    /// Reads one UTF-8 file under an additional ceiling for this operation.
    ///
    /// The additional ceiling cannot widen the session or shared job limits.
    /// It is applied by the bounded reader before the complete body is
    /// materialized.
    pub fn read_utf8_within_limits(
        &mut self,
        request: IncludeFilesystemPathRequest,
        limits: FilesystemReadLimits,
    ) -> IncludeFilesystemLimitedOutcome {
        let IncludeFilesystemPathRequest { source_id, path } = request;
        match self
            .draft_mut()
            .read_utf8_within_limits(source_id.clone(), &path, limits)
        {
            Ok(FilesystemLimitedReadOutcome::Read(outcome)) => match map_read(outcome) {
                IncludeFilesystemOutcome::Found(found) => {
                    IncludeFilesystemLimitedOutcome::Found(found)
                }
                IncludeFilesystemOutcome::NotFound(missing) => {
                    IncludeFilesystemLimitedOutcome::NotFound(missing)
                }
                IncludeFilesystemOutcome::Failed(_) => unreachable!("successful read mapping"),
            },
            Ok(FilesystemLimitedReadOutcome::AdditionalLimit) => {
                IncludeFilesystemLimitedOutcome::Limit {
                    source_id,
                    cause: IncludeFilesystemReadLimit::Additional,
                }
            }
            Ok(FilesystemLimitedReadOutcome::EstablishedLimit(error)) => {
                IncludeFilesystemLimitedOutcome::Limit {
                    source_id,
                    cause: IncludeFilesystemReadLimit::Established(error),
                }
            }
            Err(error) => IncludeFilesystemLimitedOutcome::Failed(FailedIncludeFilesystemSource {
                source_id,
                error,
            }),
        }
    }

    /// Reads a policy-bearing UTF-8 file while rejecting every symbolic link.
    pub fn read_utf8_no_symlinks_within_budget(
        &mut self,
        request: IncludeFilesystemPathRequest,
    ) -> IncludeFilesystemBudgetedOutcome {
        let IncludeFilesystemPathRequest { source_id, path } = request;
        match self
            .draft_mut()
            .read_utf8_no_symlinks_within_budget(source_id.clone(), &path)
        {
            Ok(Some(outcome)) => match map_read(outcome) {
                IncludeFilesystemOutcome::Found(found) => {
                    IncludeFilesystemBudgetedOutcome::Found(found)
                }
                IncludeFilesystemOutcome::NotFound(missing) => {
                    IncludeFilesystemBudgetedOutcome::NotFound(missing)
                }
                IncludeFilesystemOutcome::Failed(_) => unreachable!("successful read mapping"),
            },
            Ok(None) => IncludeFilesystemBudgetedOutcome::BudgetExhausted { source_id },
            Err(error) => IncludeFilesystemBudgetedOutcome::Failed(FailedIncludeFilesystemSource {
                source_id,
                error,
            }),
        }
    }

    pub fn inspect(
        &mut self,
        request: IncludeFilesystemRequest,
    ) -> IncludeFilesystemInspectionOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        let outcome = self
            .draft_mut()
            .inspect_target_outcome(source_id.clone(), &base, &target);
        map_inspection(source_id, outcome)
    }

    /// Inspects a local target below one explicitly selected authority root.
    pub fn inspect_within(
        &mut self,
        authority: &Path,
        request: IncludeFilesystemRequest,
    ) -> IncludeFilesystemInspectionOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        let outcome = self.draft_mut().inspect_target_within_outcome(
            source_id.clone(),
            authority,
            &base,
            &target,
        );
        map_inspection(source_id, outcome)
    }

    pub fn release(
        &mut self,
        binding: &IncludeFilesystemBinding,
    ) -> Result<(), FilesystemDraftError> {
        self.draft_mut().release_binding(&binding.0).map(|_| ())
    }

    pub fn usage(&self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        self.coordinator.usage()
    }

    /// Checks that this candidate can still replace the given live session.
    ///
    /// Multi-session consumers use this as a preflight before committing any
    /// member of a batch. [`Self::commit`] repeats the validation.
    pub fn validate(&self, session: &LocalFilesystemSession) -> Result<(), FilesystemDraftError> {
        self.draft_ref().validate(session)
    }

    /// Installs this draft. The shared job is finished separately by its owner.
    pub fn commit(
        mut self,
        session: &mut LocalFilesystemSession,
    ) -> Result<(), FilesystemDraftError> {
        self.draft
            .take()
            .expect("an open include transaction owns its draft")
            .prepare_commit(session)?
            .commit()
    }

    fn draft_mut(&mut self) -> &mut LocalFilesystemDraft {
        self.draft
            .as_mut()
            .expect("an open include transaction owns its draft")
    }

    fn draft_ref(&self) -> &LocalFilesystemDraft {
        self.draft
            .as_ref()
            .expect("an open include transaction owns its draft")
    }
}

fn map_live_read(
    source_id: LogicalSourceId,
    outcome: Result<FilesystemReadOutcome, ResourceError>,
) -> IncludeFilesystemOutcome {
    map_draft_read(source_id, outcome.map_err(FilesystemDraftError::from))
}

fn map_draft_read(
    source_id: LogicalSourceId,
    outcome: Result<FilesystemReadOutcome, FilesystemDraftError>,
) -> IncludeFilesystemOutcome {
    match outcome {
        Ok(outcome) => map_read(outcome),
        Err(error) => {
            IncludeFilesystemOutcome::Failed(FailedIncludeFilesystemSource { source_id, error })
        }
    }
}

fn map_read(outcome: FilesystemReadOutcome) -> IncludeFilesystemOutcome {
    match outcome {
        FilesystemReadOutcome::Found(loaded) => {
            let canonical_path = loaded.canonical_path().to_owned();
            let candidate_path = loaded.binding().candidate_path().to_owned();
            let (source_id, source, binding) = loaded.into_parts_with_binding();
            IncludeFilesystemOutcome::Found(IncludeFilesystemSource {
                source_id,
                source,
                provenance: IncludeFilesystemProvenance {
                    canonical_path: canonical_path.clone(),
                },
                binding: IncludeFilesystemBinding(binding),
                watch_candidates: watch_candidates([candidate_path, canonical_path]),
            })
        }
        FilesystemReadOutcome::NotFound {
            source_id,
            candidate_path,
        } => IncludeFilesystemOutcome::NotFound(MissingIncludeFilesystemSource {
            source_id,
            watch_candidate: IncludeWatchCandidate(candidate_path),
        }),
    }
}

fn map_inspection(
    source_id: LogicalSourceId,
    outcome: Result<FilesystemInspectOutcome, FilesystemDraftError>,
) -> IncludeFilesystemInspectionOutcome {
    match outcome {
        Ok(FilesystemInspectOutcome::Found {
            source_id,
            candidate_path,
            canonical_path,
        }) => IncludeFilesystemInspectionOutcome::Found(IncludeFilesystemInspection {
            source_id,
            provenance: IncludeFilesystemProvenance {
                canonical_path: canonical_path.clone(),
            },
            watch_candidates: watch_candidates([candidate_path, canonical_path]),
        }),
        Ok(FilesystemInspectOutcome::NotFound {
            source_id,
            candidate_path,
        }) => IncludeFilesystemInspectionOutcome::NotFound(MissingIncludeFilesystemSource {
            source_id,
            watch_candidate: IncludeWatchCandidate(candidate_path),
        }),
        Err(error) => IncludeFilesystemInspectionOutcome::Failed(FailedIncludeFilesystemSource {
            source_id,
            error,
        }),
    }
}

fn watch_candidates(paths: impl IntoIterator<Item = PathBuf>) -> Vec<IncludeWatchCandidate> {
    paths
        .into_iter()
        .map(IncludeWatchCandidate)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests;
