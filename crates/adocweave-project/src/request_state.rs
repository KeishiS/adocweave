//! Request-local filesystem observations and resource accounting.
//!
//! `RequestState` reuses an observation only under the same opened filesystem
//! authority and charges each observed file once against both request and
//! resolved-configuration limits. The state is owned by one project request
//! and is dropped with that request.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adocweave_core::SourceId;

use crate::filesystem::{FilesystemAuthority, FilesystemError, RootAuthority};
use crate::{
    ProjectLimit, ProjectLimits, ProjectResourceFailure, ProjectResourceKind,
    ProjectResourceLimits, ProjectResourceOutcome, ProjectResourceResult, ProjectSourceLocation,
};

#[derive(Default)]
pub(super) struct RequestState {
    fixed: BTreeMap<SourceId, Vec<FixedResource>>,
    inspections: BTreeMap<SourceId, Vec<FixedInspection>>,
    scopes: BTreeMap<PathBuf, ScopeBudget>,
    inspection_scopes: BTreeMap<PathBuf, ScopeBudget>,
    usage: RequestFilesystemUsage,
}

#[derive(Clone, Debug)]
pub(super) struct FixedResource {
    pub(super) source_id: SourceId,
    pub(super) requested_path: PathBuf,
    pub(super) path: PathBuf,
    pub(super) base: Option<PathBuf>,
    pub(super) no_symlinks: bool,
    pub(super) authority: Option<RootAuthority>,
    pub(super) outcome: ProjectResourceOutcome,
    pub(super) origin: crate::ProjectResourceOrigin,
    pub(super) observed_bytes: u64,
}

#[derive(Clone, Debug)]
pub(super) struct FixedInspection {
    pub(super) source_id: SourceId,
    pub(super) requested_path: PathBuf,
    pub(super) path: PathBuf,
    pub(super) authority: Option<RootAuthority>,
    pub(super) outcome: ProjectResourceOutcome,
}

pub(super) struct FixedReadRequest {
    pub(super) source_id: SourceId,
    pub(super) path: PathBuf,
    pub(super) allowance: Option<ScopeReadAllowance>,
    pub(super) no_symlinks: bool,
}

pub(super) struct InspectionRequest<'request> {
    pub(super) source_id: SourceId,
    pub(super) path: PathBuf,
    pub(super) authority: &'request Path,
    pub(super) base: &'request Path,
    pub(super) target: &'request str,
}

#[derive(Default)]
struct RequestFilesystemUsage {
    read_operations: u64,
    read_bytes: u64,
    limit: Option<ProjectLimit>,
    observations: BTreeMap<SourceId, Vec<Option<RootAuthority>>>,
}

impl RequestFilesystemUsage {
    fn reserve(
        &mut self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        max_files: usize,
    ) -> Result<(), ProjectLimit> {
        if self
            .observations
            .get(source_id)
            .is_some_and(|observations| {
                observations
                    .iter()
                    .any(|observed| same_authority(observed.as_ref(), authority))
            })
        {
            return Ok(());
        }
        if self.read_operations >= u64::try_from(max_files).unwrap_or(u64::MAX) {
            let limit = ProjectLimit::Files { limit: max_files };
            self.limit = Some(limit);
            return Err(limit);
        }
        self.observations
            .entry(source_id.clone())
            .or_default()
            .push(authority.cloned());
        self.read_operations = self.read_operations.saturating_add(1);
        Ok(())
    }
}

impl FixedInspection {
    pub(super) fn result(&self, requested_by: Option<SourceId>) -> ProjectResourceResult {
        ProjectResourceResult {
            source_id: SourceId::new(self.source_id.as_str()),
            path: self.path.clone(),
            requested_path: self.requested_path.clone(),
            kind: ProjectResourceKind::LocalTarget,
            origin: crate::ProjectResourceOrigin::Filesystem,
            requested_at: requested_by.map(|value| ProjectSourceLocation {
                source_id: SourceId::new(value.as_str()),
                range: None,
            }),
            observation: observation_candidate(
                &self.outcome,
                &self.requested_path,
                crate::ProjectResourceOrigin::Filesystem,
                crate::ProjectObservationKind::Existence,
                self.authority.is_some(),
            ),
            outcome: self.outcome.clone(),
        }
    }
}

impl FixedResource {
    pub(super) fn result(
        &self,
        kind: ProjectResourceKind,
        requested_by: Option<SourceId>,
        request_range: Option<adocweave_core::text::TextRange>,
    ) -> ProjectResourceResult {
        ProjectResourceResult {
            source_id: SourceId::new(self.source_id.as_str()),
            path: self.path.clone(),
            requested_path: self.requested_path.clone(),
            kind,
            origin: self.origin,
            requested_at: requested_by.map(|value| ProjectSourceLocation {
                source_id: SourceId::new(value.as_str()),
                range: request_range,
            }),
            observation: observation_candidate(
                &self.outcome,
                &self.requested_path,
                self.origin,
                if self.no_symlinks {
                    crate::ProjectObservationKind::ContentsNoSymlinks
                } else {
                    crate::ProjectObservationKind::Contents
                },
                self.authority.is_some(),
            ),
            outcome: self.outcome.clone(),
        }
    }
}

impl RequestState {
    pub(super) fn ensure_scope(&mut self, scope: PathBuf, limits: ProjectResourceLimits) {
        self.scopes
            .entry(scope.clone())
            .or_insert_with(|| ScopeBudget::new(limits));
        self.inspection_scopes
            .entry(scope)
            .or_insert_with(|| ScopeBudget::new(limits));
    }

    pub(super) fn reserve_inspection_scope(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
    ) -> Result<(), ProjectLimit> {
        self.inspection_scopes
            .get_mut(scope)
            .expect("a resolved configuration creates its inspection budget")
            .reserve(source_id, authority)
    }

    pub(super) fn reserve_scope(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
    ) -> Result<(), ProjectLimit> {
        self.scopes
            .get_mut(scope)
            .expect("a resolved configuration creates its scope budget")
            .reserve(source_id, authority)
    }

    pub(super) fn charge_scope_body(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        bytes: usize,
    ) -> Result<(), ProjectLimit> {
        self.scopes
            .get_mut(scope)
            .expect("a resolved configuration creates its scope budget")
            .charge_body(source_id, authority, bytes)
    }

    pub(super) fn scope_read_allowance(
        &self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        request_limits: ProjectResourceLimits,
        filesystem_limits: ProjectResourceLimits,
    ) -> Result<ScopeReadAllowance, ProjectLimit> {
        let request_remaining_bytes = request_limits
            .max_total_bytes
            .saturating_sub(self.usage.read_bytes);
        self.scopes
            .get(scope)
            .expect("a resolved configuration creates its scope budget")
            .read_allowance(
                source_id,
                authority,
                filesystem_limits,
                request_remaining_bytes,
            )
    }

    pub(super) fn fix_scope_read_limit(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        fixed: &FixedResource,
        allowance: ScopeReadAllowance,
    ) {
        if fixed.observed_bytes > 0
            && let Err(limit) = self
                .scopes
                .get_mut(scope)
                .expect("a resolved configuration creates its scope budget")
                .charge_observed_bytes(source_id, authority, fixed.observed_bytes)
        {
            self.scopes
                .get_mut(scope)
                .expect("a resolved configuration creates its scope budget")
                .fix_limit(source_id, authority, limit);
        }
        if allowance.scope_specific
            && matches!(
                fixed.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit))
                    if limit == allowance.limit
            )
        {
            self.scopes
                .get_mut(scope)
                .expect("a resolved configuration creates its scope budget")
                .fix_limit(source_id, authority, allowance.limit);
        }
    }

    pub(super) fn read(
        &mut self,
        limits: ProjectLimits,
        request: FixedReadRequest,
        filesystem: &FilesystemAuthority,
    ) -> FixedResource {
        let FixedReadRequest {
            source_id,
            path,
            allowance,
            no_symlinks,
        } = request;
        if let Some(fixed) = self
            .fixed
            .get(&source_id)
            .and_then(|fixed| reusable_resource(fixed, &path, filesystem, no_symlinks))
        {
            return fixed;
        }
        let authority = filesystem.authority_for_path(&path).cloned();
        if let Err(limit) =
            self.usage
                .reserve(&source_id, authority.as_ref(), limits.resources.max_files)
        {
            return limited_resource(source_id, path, no_symlinks, authority, limit);
        }
        if let Ok(canonical) = filesystem.inspect(&path, no_symlinks)
            && let Some(source) = self.fixed.values().flatten().find_map(|fixed| {
                (fixed.path == canonical
                    && (!no_symlinks || fixed.no_symlinks)
                    && same_authority(
                        fixed.authority.as_ref(),
                        filesystem.authority_for_path(&path),
                    ))
                .then(|| match &fixed.outcome {
                    ProjectResourceOutcome::Loaded { source } => Some(Arc::clone(source)),
                    _ => None,
                })?
            })
        {
            let fixed = FixedResource {
                source_id: source_id.clone(),
                requested_path: path.clone(),
                path: canonical.clone(),
                base: canonical.parent().map(Path::to_owned),
                no_symlinks,
                authority: filesystem.authority_for_path(&path).cloned(),
                outcome: ProjectResourceOutcome::Loaded { source },
                origin: crate::ProjectResourceOrigin::Filesystem,
                observed_bytes: 0,
            };
            self.fixed.entry(source_id).or_default().push(fixed.clone());
            return fixed;
        }
        if let Some(inspection) = self
            .inspections
            .get(&source_id)
            .and_then(|fixed| reusable_inspection(fixed, &path, filesystem))
            .filter(|inspection| matches!(inspection.outcome, ProjectResourceOutcome::Missing))
        {
            let fixed = FixedResource {
                source_id: source_id.clone(),
                requested_path: path,
                path: inspection.path,
                base: None,
                no_symlinks,
                authority: inspection.authority,
                outcome: ProjectResourceOutcome::Missing,
                origin: crate::ProjectResourceOrigin::Filesystem,
                observed_bytes: 0,
            };
            self.fixed.entry(source_id).or_default().push(fixed.clone());
            return fixed;
        }

        let requested_path = path.clone();
        let (max_bytes, exhausted_limit) = allowance.map_or_else(
            || {
                let remaining = limits
                    .resources
                    .max_total_bytes
                    .saturating_sub(self.usage.read_bytes);
                if limits.resources.max_resource_bytes <= remaining {
                    (
                        limits.resources.max_resource_bytes,
                        ProjectLimit::ResourceBytes {
                            limit: limits.resources.max_resource_bytes,
                        },
                    )
                } else {
                    (
                        remaining,
                        ProjectLimit::ReadBytes {
                            limit: limits.resources.max_total_bytes,
                        },
                    )
                }
            },
            |allowance| (allowance.max_bytes, allowance.limit),
        );
        let mut loaded_path = None;
        let mut observed_bytes = 0;
        let outcome = if max_bytes == 0 {
            if allowance.is_none_or(|allowance| !allowance.scope_specific) {
                self.usage.limit = Some(exhausted_limit);
            }
            ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(exhausted_limit))
        } else {
            let read = filesystem.read_utf8(&path, max_bytes, no_symlinks);
            observed_bytes = read.observed_bytes;
            self.usage.read_bytes = self.usage.read_bytes.saturating_add(read.observed_bytes);
            match read.outcome {
                Ok(loaded) => {
                    loaded_path = Some(loaded.canonical_path().to_owned());
                    ProjectResourceOutcome::Loaded {
                        source: Arc::from(loaded.source()),
                    }
                }
                Err(FilesystemError::Missing(_)) => ProjectResourceOutcome::Missing,
                Err(FilesystemError::ResourceTooLarge(_)) => {
                    if matches!(exhausted_limit, ProjectLimit::ReadBytes { .. })
                        && allowance.is_none_or(|allowance| !allowance.scope_specific)
                    {
                        self.usage.limit = Some(exhausted_limit);
                    }
                    ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(exhausted_limit))
                }
                Err(error) => {
                    ProjectResourceOutcome::Failed(classify_resource_failure(error, limits))
                }
            }
        };
        let resolved_path = loaded_path.unwrap_or_else(|| path.clone());
        let base = matches!(outcome, ProjectResourceOutcome::Loaded { .. })
            .then(|| resolved_path.parent().map(Path::to_owned))
            .flatten();
        let fixed = FixedResource {
            source_id: source_id.clone(),
            requested_path,
            path: resolved_path,
            base,
            no_symlinks,
            authority,
            outcome,
            origin: crate::ProjectResourceOrigin::Filesystem,
            observed_bytes,
        };
        let scope_limited = allowance.is_some_and(|allowance| {
            allowance.scope_specific
                && matches!(
                    fixed.outcome,
                    ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit))
                        if limit == allowance.limit
                )
        });
        if fixed.authority.is_some() && !scope_limited {
            self.fixed.entry(source_id).or_default().push(fixed.clone());
            observe_fixed(&fixed);
        }
        fixed
    }

    pub(super) fn inspect(
        &mut self,
        scope: &Path,
        request: InspectionRequest<'_>,
        filesystem: &mut FilesystemAuthority,
        filesystem_limits: ProjectLimits,
        request_limits: ProjectLimits,
    ) -> FixedInspection {
        let InspectionRequest {
            source_id,
            path,
            authority,
            base,
            target,
        } = request;
        let budget_authority = filesystem.authority_for_path(&path).cloned();
        if let Err(limit) =
            self.reserve_inspection_scope(scope, &source_id, budget_authority.as_ref())
        {
            return limited_inspection(source_id, path, limit);
        }
        if let Some(fixed) = self
            .inspections
            .get(&source_id)
            .and_then(|fixed| reusable_inspection(fixed, &path, filesystem))
        {
            return fixed;
        }
        if let Some(fixed) = self
            .fixed
            .get(&source_id)
            .and_then(|fixed| reusable_resource(fixed, &path, filesystem, false))
        {
            let outcome = match &fixed.outcome {
                ProjectResourceOutcome::Loaded { .. }
                | ProjectResourceOutcome::LoadedOmitted { .. } => {
                    Some(ProjectResourceOutcome::Present)
                }
                ProjectResourceOutcome::Missing => Some(ProjectResourceOutcome::Missing),
                ProjectResourceOutcome::Present | ProjectResourceOutcome::Failed(_) => None,
            };
            if let Some(outcome) = outcome {
                let inspection = FixedInspection {
                    source_id: source_id.clone(),
                    requested_path: path,
                    path: fixed.path,
                    authority: fixed.authority,
                    outcome,
                };
                self.inspections
                    .entry(source_id)
                    .or_default()
                    .push(inspection.clone());
                return inspection;
            }
        }
        let requested_path = path.clone();
        let acquired_authority = filesystem.authority_for_path(&path).cloned();
        if let Err(limit) = self.usage.reserve(
            &source_id,
            acquired_authority.as_ref(),
            filesystem_limits.resources.max_files,
        ) {
            return limited_inspection(source_id, path, limit);
        }
        let outcome = filesystem
            .authority_for_path(authority)
            .ok_or_else(|| FilesystemError::OutsideRoot(path.clone()))
            .and_then(|authority| authority.inspect(base, target))
            .map_or_else(
                |error| match error {
                    FilesystemError::Missing(_) => {
                        Ok((path.clone(), ProjectResourceOutcome::Missing))
                    }
                    error => Err(error),
                },
                |canonical| Ok((canonical, ProjectResourceOutcome::Present)),
            );
        let (path, outcome) = match outcome {
            Ok(value) => value,
            Err(error) => (
                path,
                ProjectResourceOutcome::Failed(classify_resource_failure(error, request_limits)),
            ),
        };
        let fixed = FixedInspection {
            source_id: source_id.clone(),
            requested_path,
            path: path.clone(),
            authority: acquired_authority,
            outcome: outcome.clone(),
        };
        if fixed.authority.is_some() {
            self.inspections
                .entry(source_id)
                .or_default()
                .push(fixed.clone());
        }
        fixed
    }

    pub(super) fn reusable_resource(
        &self,
        source_id: &SourceId,
        path: &Path,
        filesystem: &FilesystemAuthority,
        no_symlinks: bool,
    ) -> Option<FixedResource> {
        self.fixed
            .get(source_id)
            .and_then(|fixed| reusable_resource(fixed, path, filesystem, no_symlinks))
    }

    pub(super) fn write_authority(
        &self,
        source_id: &SourceId,
        path: &Path,
    ) -> Option<RootAuthority> {
        self.fixed.get(source_id).and_then(|fixed| {
            fixed.iter().find_map(|fixed| {
                (fixed.requested_path == path
                    && matches!(fixed.outcome, ProjectResourceOutcome::Loaded { .. }))
                .then(|| fixed.authority.clone())
                .flatten()
            })
        })
    }

    pub(super) fn config_observations(&self) -> Vec<ProjectResourceResult> {
        let mut resources = self
            .fixed
            .values()
            .flatten()
            .filter(|fixed| {
                fixed
                    .requested_path
                    .file_name()
                    .is_some_and(|name| name == crate::config::FILE_NAME)
            })
            .map(|fixed| fixed.result(ProjectResourceKind::Config, None, None))
            .collect::<Vec<_>>();
        resources.sort_by(|left, right| left.requested_path.cmp(&right.requested_path));
        resources.dedup_by(|left, right| {
            left.source_id == right.source_id && left.requested_path == right.requested_path
        });
        resources
    }

    pub(super) const fn limit(&self) -> Option<ProjectLimit> {
        self.usage.limit
    }

    pub(super) const fn read_operations(&self) -> u64 {
        self.usage.read_operations
    }

    pub(super) const fn read_bytes(&self) -> u64 {
        self.usage.read_bytes
    }
}

fn reusable_resource(
    fixed: &[FixedResource],
    requested_path: &Path,
    filesystem: &FilesystemAuthority,
    no_symlinks: bool,
) -> Option<FixedResource> {
    fixed.iter().find_map(|fixed| {
        (validate_authority(
            requested_path,
            &fixed.requested_path,
            &fixed.path,
            matches!(
                fixed.outcome,
                ProjectResourceOutcome::Loaded { .. }
                    | ProjectResourceOutcome::LoadedOmitted { .. }
                    | ProjectResourceOutcome::Present
            ),
            fixed.authority.as_ref(),
            filesystem,
        )
        .is_ok()
            && fixed.no_symlinks == no_symlinks)
            .then(|| fixed.clone())
    })
}

fn reusable_inspection(
    fixed: &[FixedInspection],
    requested_path: &Path,
    filesystem: &FilesystemAuthority,
) -> Option<FixedInspection> {
    fixed.iter().find_map(|fixed| {
        validate_authority(
            requested_path,
            &fixed.requested_path,
            &fixed.path,
            matches!(fixed.outcome, ProjectResourceOutcome::Present),
            fixed.authority.as_ref(),
            filesystem,
        )
        .is_ok()
        .then(|| fixed.clone())
    })
}

pub(super) fn validate_authority(
    requested_path: &Path,
    acquired_requested_path: &Path,
    acquired_path: &Path,
    acquired_path_is_verified: bool,
    acquired_authority: Option<&RootAuthority>,
    filesystem: &FilesystemAuthority,
) -> Result<(), FilesystemError> {
    let requested_authority = filesystem.authority_for_path(requested_path);
    let same_authority = requested_authority
        .zip(acquired_authority)
        .is_some_and(|(requested, acquired)| requested.has_same_authority(acquired));
    let acquired_paths_are_valid = acquired_authority.is_some_and(|authority| {
        authority.contains_path(acquired_requested_path)
            && (!acquired_path_is_verified || authority.contains_path(acquired_path))
    });
    if !same_authority || !acquired_paths_are_valid {
        return Err(FilesystemError::OutsideRoot(requested_path.to_owned()));
    }
    Ok(())
}

struct ScopeBudget {
    limits: ProjectResourceLimits,
    observations: BTreeMap<SourceId, Vec<BudgetObservation>>,
    files: usize,
    bytes: u64,
}

struct BudgetObservation {
    authority: Option<RootAuthority>,
    observed_bytes: u64,
    observed_bytes_charged: bool,
    body_charged: bool,
    limit: Option<ProjectLimit>,
}

#[derive(Clone, Copy)]
pub(super) struct ScopeReadAllowance {
    max_bytes: u64,
    limit: ProjectLimit,
    scope_specific: bool,
}

impl ScopeBudget {
    fn new(limits: ProjectResourceLimits) -> Self {
        Self {
            limits,
            observations: BTreeMap::new(),
            files: 0,
            bytes: 0,
        }
    }

    fn reserve(
        &mut self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
    ) -> Result<(), ProjectLimit> {
        if self.observation(source_id, authority).is_some() {
            return Ok(());
        }
        if self.files >= self.limits.max_files {
            return Err(ProjectLimit::Files {
                limit: self.limits.max_files,
            });
        }
        self.observations
            .entry(source_id.clone())
            .or_default()
            .push(BudgetObservation {
                authority: authority.cloned(),
                observed_bytes: 0,
                observed_bytes_charged: false,
                body_charged: false,
                limit: None,
            });
        self.files += 1;
        Ok(())
    }

    fn charge_observed_bytes(
        &mut self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        bytes: u64,
    ) -> Result<(), ProjectLimit> {
        let Some(observations) = self.observations.get_mut(source_id) else {
            unreachable!("resource bodies are charged only after reserving their observation")
        };
        let Some(observation) = observations
            .iter_mut()
            .find(|observation| same_authority(observation.authority.as_ref(), authority))
        else {
            unreachable!("resource bodies are charged only under their reserved authority")
        };
        if observation.observed_bytes_charged {
            return Ok(());
        }
        let previous_bytes = self.bytes;
        self.bytes = self.bytes.saturating_add(bytes);
        observation.observed_bytes = bytes;
        observation.observed_bytes_charged = true;
        if bytes > self.limits.max_resource_bytes {
            let limit = ProjectLimit::ResourceBytes {
                limit: self.limits.max_resource_bytes,
            };
            observation.limit = Some(limit);
            return Err(limit);
        }
        if previous_bytes.saturating_add(bytes) > self.limits.max_total_bytes {
            let limit = ProjectLimit::ReadBytes {
                limit: self.limits.max_total_bytes,
            };
            observation.limit = Some(limit);
            return Err(limit);
        }
        Ok(())
    }

    fn charge_body(
        &mut self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        bytes: usize,
    ) -> Result<(), ProjectLimit> {
        let Some(observations) = self.observations.get_mut(source_id) else {
            unreachable!("resource bodies are charged only after reserving their observation")
        };
        let Some(observation) = observations
            .iter_mut()
            .find(|observation| same_authority(observation.authority.as_ref(), authority))
        else {
            unreachable!("resource bodies are charged only under their reserved authority")
        };
        if observation.body_charged {
            return Ok(());
        }
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        let unaccounted = bytes.saturating_sub(observation.observed_bytes);
        let previous_bytes = self.bytes;
        self.bytes = self.bytes.saturating_add(unaccounted);
        observation.body_charged = true;
        if bytes > self.limits.max_resource_bytes {
            let limit = ProjectLimit::ResourceBytes {
                limit: self.limits.max_resource_bytes,
            };
            observation.limit = Some(limit);
            return Err(limit);
        }
        if previous_bytes.saturating_add(unaccounted) > self.limits.max_total_bytes {
            let limit = ProjectLimit::ReadBytes {
                limit: self.limits.max_total_bytes,
            };
            observation.limit = Some(limit);
            return Err(limit);
        }
        Ok(())
    }

    fn observation(
        &self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
    ) -> Option<&BudgetObservation> {
        self.observations
            .get(source_id)?
            .iter()
            .find(|observation| same_authority(observation.authority.as_ref(), authority))
    }

    fn read_allowance(
        &self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        request_limits: ProjectResourceLimits,
        request_remaining_bytes: u64,
    ) -> Result<ScopeReadAllowance, ProjectLimit> {
        let observation = self
            .observation(source_id, authority)
            .expect("a read allowance follows a reserved observation");
        if let Some(limit) = observation.limit {
            return Err(limit);
        }
        let resource_bytes = self
            .limits
            .max_resource_bytes
            .min(request_limits.max_resource_bytes);
        let scope_remaining_bytes = if observation.body_charged {
            resource_bytes
        } else {
            self.limits.max_total_bytes.saturating_sub(self.bytes)
        };
        if scope_remaining_bytes == 0 {
            return Err(ProjectLimit::ReadBytes {
                limit: self.limits.max_total_bytes,
            });
        }
        let remaining_total = scope_remaining_bytes.min(request_remaining_bytes);
        let (limit, scope_specific) = if resource_bytes <= remaining_total {
            let scope_specific = self.limits.max_resource_bytes < request_limits.max_resource_bytes;
            let limit = if scope_specific {
                self.limits.max_resource_bytes
            } else {
                request_limits.max_resource_bytes
            };
            (ProjectLimit::ResourceBytes { limit }, scope_specific)
        } else if scope_remaining_bytes < request_remaining_bytes {
            (
                ProjectLimit::ReadBytes {
                    limit: self.limits.max_total_bytes,
                },
                true,
            )
        } else {
            (
                ProjectLimit::ReadBytes {
                    limit: request_limits.max_total_bytes,
                },
                false,
            )
        };
        Ok(ScopeReadAllowance {
            max_bytes: remaining_total.min(resource_bytes),
            limit,
            scope_specific,
        })
    }

    fn fix_limit(
        &mut self,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        limit: ProjectLimit,
    ) {
        let observation = self
            .observations
            .get_mut(source_id)
            .and_then(|observations| {
                observations
                    .iter_mut()
                    .find(|observation| same_authority(observation.authority.as_ref(), authority))
            })
            .expect("a fixed limit follows a reserved observation");
        observation.limit = Some(limit);
    }
}

fn same_authority(left: Option<&RootAuthority>, right: Option<&RootAuthority>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.has_same_authority(right),
        (None, None) => true,
        _ => false,
    }
}

pub(super) fn limited_resource(
    source_id: SourceId,
    path: PathBuf,
    no_symlinks: bool,
    authority: Option<RootAuthority>,
    limit: ProjectLimit,
) -> FixedResource {
    FixedResource {
        source_id,
        requested_path: path.clone(),
        path,
        base: None,
        no_symlinks,
        authority,
        outcome: ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)),
        origin: crate::ProjectResourceOrigin::Filesystem,
        observed_bytes: 0,
    }
}

fn limited_inspection(source_id: SourceId, path: PathBuf, limit: ProjectLimit) -> FixedInspection {
    FixedInspection {
        source_id,
        requested_path: path.clone(),
        path,
        authority: None,
        outcome: ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)),
    }
}

fn classify_resource_failure(
    error: FilesystemError,
    limits: ProjectLimits,
) -> ProjectResourceFailure {
    match &error {
        FilesystemError::LimitExceeded { limit } => {
            ProjectResourceFailure::Limit(ProjectLimit::Files { limit: *limit })
        }
        FilesystemError::ReadLimitExceeded => {
            ProjectResourceFailure::Limit(ProjectLimit::ReadBytes {
                limit: limits.resources.max_total_bytes,
            })
        }
        FilesystemError::ResourceTooLarge(_) => {
            ProjectResourceFailure::Limit(ProjectLimit::ResourceBytes {
                limit: limits.resources.max_resource_bytes,
            })
        }
        FilesystemError::PermissionDenied(_) | FilesystemError::InvalidUtf8(_) => {
            ProjectResourceFailure::Unreadable(crate::ProjectResourceError::from_filesystem(error))
        }
        _ => ProjectResourceFailure::Rejected(crate::ProjectResourceError::from_filesystem(error)),
    }
}

pub(super) fn observation_candidate(
    outcome: &ProjectResourceOutcome,
    path: &Path,
    origin: crate::ProjectResourceOrigin,
    kind: crate::ProjectObservationKind,
    safely_repeat_rejection: bool,
) -> Option<crate::ProjectObservationCandidate> {
    if origin == crate::ProjectResourceOrigin::Input {
        return None;
    }
    let observable = match outcome {
        ProjectResourceOutcome::Loaded { .. }
        | ProjectResourceOutcome::LoadedOmitted { .. }
        | ProjectResourceOutcome::Present
        | ProjectResourceOutcome::Missing => true,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Unreadable(_)) => true,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_)) => {
            safely_repeat_rejection
        }
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(_)) => false,
    };
    observable.then(|| crate::ProjectObservationCandidate {
        path: path.to_owned(),
        kind,
        observation: crate::ProjectResourceObservation::from_outcome(outcome),
    })
}

#[cfg(test)]
pub(super) type FixedObserver = Option<Box<dyn FnMut(&FixedResource)>>;

#[cfg(test)]
thread_local! {
    pub(super) static FIXED_OBSERVER: std::cell::RefCell<FixedObserver> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn observe_fixed(fixed: &FixedResource) {
    FIXED_OBSERVER.with(|observer| {
        if let Some(observer) = observer.borrow_mut().as_mut() {
            observer(fixed);
        }
    });
}

#[cfg(not(test))]
fn observe_fixed(_: &FixedResource) {}
