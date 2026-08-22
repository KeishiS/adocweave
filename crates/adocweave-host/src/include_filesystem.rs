//! Transactional filesystem boundary for native include drivers.
//!
//! The lower-level local-resource API remains available for host internals and
//! security tests. Include consumers use this module so draft ownership, job
//! accounting and generation-specific resource bindings cannot be separated.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::local_resource::FilesystemInspectOutcome;
use crate::{
    FilesystemDraftError, FilesystemJobCoordinator, FilesystemJobError, FilesystemJobLimits,
    FilesystemJobUsage, FilesystemReadOutcome, FilesystemResourceBinding, LocalFilesystemDraft,
    LocalFilesystemSession, LogicalSourceId, ResourceError,
};

/// One authored include lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemRequest {
    source_id: LogicalSourceId,
    base: PathBuf,
    target: Arc<str>,
}

/// Logical namespace which owns one committed set of include bindings.
///
/// A workspace root, CLI input or another host run can use a distinct owner so
/// replacing its include set never releases bindings retained by another
/// consumer of the same [`LocalFilesystemSession`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IncludeFilesystemOwner(LogicalSourceId);

impl IncludeFilesystemOwner {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, ResourceError> {
        LogicalSourceId::new(value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
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

/// A path which was derived through an authorized, normalized lookup.
///
/// Values cannot be constructed by callers. A missing target contributes its
/// normalized candidate. A found target contributes both that candidate and
/// its canonical path, with duplicates removed.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IncludeWatchCandidate(PathBuf);

impl IncludeWatchCandidate {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Canonical identity of a file opened through the retained root authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemProvenance {
    canonical_path: PathBuf,
}

impl IncludeFilesystemProvenance {
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }
}

/// A successfully loaded include source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemSource {
    source_id: LogicalSourceId,
    source: Arc<str>,
    provenance: IncludeFilesystemProvenance,
    watch_candidates: Vec<IncludeWatchCandidate>,
}

/// A verified local target whose contents were not read.
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

    pub fn watch_candidates(&self) -> &[IncludeWatchCandidate] {
        &self.watch_candidates
    }

    pub fn into_parts(self) -> (LogicalSourceId, Arc<str>, IncludeFilesystemProvenance) {
        (self.source_id, self.source, self.provenance)
    }
}

/// An absent include and the normalized path which can safely be watched.
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

/// A failed include lookup.
///
/// No watch path is exposed for a failed lookup: path normalization or
/// confinement may itself be the operation which failed.
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

/// Typed result of one include lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemOutcome {
    Found(IncludeFilesystemSource),
    NotFound(MissingIncludeFilesystemSource),
    Failed(FailedIncludeFilesystemSource),
}

/// Typed result of verifying one local target without reading its contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncludeFilesystemInspectionOutcome {
    Found(IncludeFilesystemInspection),
    NotFound(MissingIncludeFilesystemSource),
    Failed(FailedIncludeFilesystemSource),
}

/// Committed observations from one include transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncludeFilesystemCommit {
    usage: FilesystemJobUsage,
    watch_candidates: Vec<IncludeWatchCandidate>,
}

impl IncludeFilesystemCommit {
    pub const fn usage(&self) -> FilesystemJobUsage {
        self.usage
    }

    pub fn watch_candidates(&self) -> &[IncludeWatchCandidate] {
        &self.watch_candidates
    }
}

/// Long-lived ownership registry for native include bindings.
///
/// Filesystem sessions stay with the host because workspace scans, watch
/// handling and other native operations can share their root authority and
/// budget. Include bindings remain private and are partitioned by
/// [`IncludeFilesystemOwner`].
#[derive(Debug)]
pub struct IncludeFilesystem {
    id: u64,
    bindings: BTreeMap<IncludeFilesystemOwner, IncludeOwnerBindings>,
}

#[derive(Clone, Debug, Default)]
struct IncludeOwnerBindings {
    revision: u64,
    resources: BTreeMap<LogicalSourceId, FilesystemResourceBinding>,
}

static NEXT_INCLUDE_FILESYSTEM_ID: AtomicU64 = AtomicU64::new(1);

impl IncludeFilesystem {
    pub fn new() -> Result<Self, ResourceError> {
        let id = NEXT_INCLUDE_FILESYSTEM_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| {
                ResourceError::Unverifiable(
                    "include filesystem identity space is exhausted".to_owned(),
                )
            })?;
        Ok(Self {
            id,
            bindings: BTreeMap::new(),
        })
    }

    /// Starts an owned replacement of include observations in this registry.
    ///
    /// The session is borrowed only while its owned draft is created. This lets
    /// a host release a session lock during parsing and provide the live session
    /// again at commit. Within every owner passed to [`IncludeFilesystemTransaction::read`],
    /// a commit releases bindings for logical IDs which were not observed.
    /// Dropping the transaction leaves the live session and registry unchanged.
    pub fn transaction(
        &self,
        session: &LocalFilesystemSession,
        limits: FilesystemJobLimits,
    ) -> Result<IncludeFilesystemTransaction, FilesystemDraftError> {
        let job = FilesystemJobCoordinator::new(limits)?;
        let draft = session.draft(&job)?;
        let candidate_bindings = self
            .bindings
            .iter()
            .map(|(owner, bindings)| (owner.clone(), bindings.resources.clone()))
            .collect();
        let base_revisions = self
            .bindings
            .iter()
            .map(|(owner, bindings)| (owner.clone(), bindings.revision))
            .collect();
        Ok(IncludeFilesystemTransaction {
            filesystem_id: self.id,
            base_revisions,
            candidate_bindings,
            seen: BTreeMap::new(),
            watch_candidates: BTreeSet::new(),
            draft: Some(draft),
            job,
        })
    }
}

/// One isolated include-filesystem replacement.
#[must_use = "an include filesystem transaction must be committed or dropped"]
pub struct IncludeFilesystemTransaction {
    filesystem_id: u64,
    base_revisions: BTreeMap<IncludeFilesystemOwner, u64>,
    candidate_bindings:
        BTreeMap<IncludeFilesystemOwner, BTreeMap<LogicalSourceId, FilesystemResourceBinding>>,
    seen: BTreeMap<IncludeFilesystemOwner, BTreeSet<LogicalSourceId>>,
    watch_candidates: BTreeSet<IncludeWatchCandidate>,
    draft: Option<LocalFilesystemDraft>,
    job: FilesystemJobCoordinator,
}

impl IncludeFilesystemTransaction {
    /// Reads one include through the handle-relative, bounded draft API.
    pub fn read(
        &mut self,
        owner: IncludeFilesystemOwner,
        request: IncludeFilesystemRequest,
    ) -> IncludeFilesystemOutcome {
        let IncludeFilesystemRequest {
            source_id,
            base,
            target,
        } = request;
        self.seen
            .entry(owner.clone())
            .or_default()
            .insert(source_id.clone());
        let outcome = self
            .draft
            .as_mut()
            .expect("an open include transaction owns its draft")
            .read_target_utf8_outcome(source_id.clone(), &base, &target);
        match outcome {
            Ok(FilesystemReadOutcome::Found(loaded)) => {
                let canonical_path = loaded.canonical_path().to_owned();
                let binding = loaded.binding().clone();
                let candidate_path = binding.candidate_path().to_owned();
                let previous = self
                    .candidate_bindings
                    .entry(owner)
                    .or_default()
                    .insert(source_id.clone(), binding);
                if let Some(previous) = previous
                    && previous.candidate_path() != candidate_path
                    && let Err(error) = self.release_binding(&previous)
                {
                    return IncludeFilesystemOutcome::Failed(FailedIncludeFilesystemSource {
                        source_id,
                        error,
                    });
                }
                let watch_candidates = watch_candidates([candidate_path, canonical_path.clone()]);
                self.watch_candidates
                    .extend(watch_candidates.iter().cloned());
                let (_, source) = loaded.into_parts();
                IncludeFilesystemOutcome::Found(IncludeFilesystemSource {
                    source_id,
                    source,
                    provenance: IncludeFilesystemProvenance { canonical_path },
                    watch_candidates,
                })
            }
            Ok(FilesystemReadOutcome::NotFound {
                source_id,
                candidate_path,
            }) => {
                let previous = self
                    .candidate_bindings
                    .entry(owner)
                    .or_default()
                    .remove(&source_id);
                if let Some(previous) = previous
                    && let Err(error) = self.release_binding(&previous)
                {
                    return IncludeFilesystemOutcome::Failed(FailedIncludeFilesystemSource {
                        source_id,
                        error,
                    });
                }
                let watch_candidate = IncludeWatchCandidate(candidate_path);
                self.watch_candidates.insert(watch_candidate.clone());
                IncludeFilesystemOutcome::NotFound(MissingIncludeFilesystemSource {
                    source_id,
                    watch_candidate,
                })
            }
            Err(error) => {
                IncludeFilesystemOutcome::Failed(FailedIncludeFilesystemSource { source_id, error })
            }
        }
    }

    /// Verifies a local target through the same authority and job as includes.
    ///
    /// Inspection consumes a read-operation and path budget but no read bytes,
    /// and it does not create or replace an include binding.
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
            .draft
            .as_mut()
            .expect("an open include transaction owns its draft")
            .inspect_target_outcome(source_id.clone(), &base, &target);
        match outcome {
            Ok(FilesystemInspectOutcome::Found {
                source_id,
                candidate_path,
                canonical_path,
            }) => {
                let watch_candidates = watch_candidates([candidate_path, canonical_path.clone()]);
                self.watch_candidates
                    .extend(watch_candidates.iter().cloned());
                IncludeFilesystemInspectionOutcome::Found(IncludeFilesystemInspection {
                    source_id,
                    provenance: IncludeFilesystemProvenance { canonical_path },
                    watch_candidates,
                })
            }
            Ok(FilesystemInspectOutcome::NotFound {
                source_id,
                candidate_path,
            }) => {
                let watch_candidate = IncludeWatchCandidate(candidate_path);
                self.watch_candidates.insert(watch_candidate.clone());
                IncludeFilesystemInspectionOutcome::NotFound(MissingIncludeFilesystemSource {
                    source_id,
                    watch_candidate,
                })
            }
            Err(error) => {
                IncludeFilesystemInspectionOutcome::Failed(FailedIncludeFilesystemSource {
                    source_id,
                    error,
                })
            }
        }
    }

    pub fn usage(&self) -> Result<FilesystemJobUsage, FilesystemJobError> {
        self.job.usage()
    }

    /// Cancels this transaction without exposing the coordinator itself.
    pub fn cancel(&self) -> Result<(), FilesystemJobError> {
        self.job.cancel()
    }

    /// Atomically installs the draft and its logical binding ownership.
    pub fn commit(
        mut self,
        session: &mut LocalFilesystemSession,
        filesystem: &mut IncludeFilesystem,
    ) -> Result<IncludeFilesystemCommit, FilesystemDraftError> {
        if self.filesystem_id != filesystem.id {
            return Err(FilesystemDraftError::InvalidDraft);
        }
        let owners = self.seen.keys().cloned().collect::<Vec<_>>();
        for owner in &owners {
            let seen = self.seen.get(owner).expect("owner was collected from seen");
            let resources = self.candidate_bindings.entry(owner.clone()).or_default();
            let unseen = resources
                .keys()
                .filter(|source_id| !seen.contains(*source_id))
                .cloned()
                .collect::<Vec<_>>();
            let released = unseen
                .into_iter()
                .filter_map(|source_id| resources.remove(&source_id))
                .collect::<Vec<_>>();
            for binding in released {
                self.release_binding(&binding)?;
            }
        }
        let mut next_revisions = BTreeMap::new();
        for owner in &owners {
            let expected = self.base_revisions.get(owner).copied().unwrap_or(0);
            let current = filesystem
                .bindings
                .get(owner)
                .map(|bindings| bindings.revision)
                .unwrap_or(0);
            if current != expected {
                return Err(FilesystemDraftError::InvalidDraft);
            }
            next_revisions.insert(
                owner.clone(),
                current
                    .checked_add(1)
                    .ok_or(FilesystemDraftError::SessionRevisionExhausted)?,
            );
        }
        let usage = self.job.usage()?;
        let draft = self
            .draft
            .take()
            .expect("an open include transaction owns its draft");
        draft.prepare_commit(session)?.commit()?;
        for owner in owners {
            let resources = self.candidate_bindings.remove(&owner).unwrap_or_default();
            filesystem.bindings.insert(
                owner.clone(),
                IncludeOwnerBindings {
                    revision: next_revisions[&owner],
                    resources,
                },
            );
        }
        self.job.finish()?;
        Ok(IncludeFilesystemCommit {
            usage,
            watch_candidates: std::mem::take(&mut self.watch_candidates)
                .into_iter()
                .collect(),
        })
    }

    fn release_binding(
        &mut self,
        binding: &FilesystemResourceBinding,
    ) -> Result<(), FilesystemDraftError> {
        self.draft
            .as_mut()
            .expect("an open include transaction owns its draft")
            .release_binding(binding)
            .map(|_| ())
    }
}

impl Drop for IncludeFilesystemTransaction {
    fn drop(&mut self) {
        if self.draft.is_some() {
            let _ = self.job.cancel();
        }
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
