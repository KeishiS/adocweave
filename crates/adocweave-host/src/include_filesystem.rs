//! High-level filesystem boundary for native include drivers.
//!
//! Atomic transactions own the draft and expose opaque bindings. Consumers
//! retain those bindings in their existing state and release obsolete values
//! explicitly. Lenient live operations are separate because one failed
//! optional target must not poison later CLI validation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::local_resource::FilesystemInspectOutcome;
use crate::{
    FilesystemDraftError, FilesystemJobCoordinator, FilesystemJobError, FilesystemJobLimits,
    FilesystemJobUsage, FilesystemReadOutcome, FilesystemResourceBinding, LocalFilesystemDraft,
    LocalFilesystemSession, LogicalSourceId, ResourceError,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemInspectionOutcome {
    Found(IncludeFilesystemInspection),
    NotFound(MissingIncludeFilesystemSource),
    Failed(FailedIncludeFilesystemSource),
}

/// Stateless entry point for lenient operations on live session state.
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

    pub fn usage(&self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        self.coordinator.usage()
    }

    pub fn cancel(&self) -> Result<(), FilesystemJobError> {
        self.coordinator.cancel()
    }

    pub fn finish(self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        let usage = self.coordinator.usage()?;
        self.coordinator.finish()?;
        Ok(usage)
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

    pub fn release(
        &mut self,
        binding: &IncludeFilesystemBinding,
    ) -> Result<(), FilesystemDraftError> {
        self.draft_mut().release_binding(&binding.0).map(|_| ())
    }

    pub fn usage(&self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        self.coordinator.usage()
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
