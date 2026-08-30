use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::LoadedProjectConfig;
use adocweave::preprocess::{
    EffectivePreprocessStep, EffectiveProcessingOptions, PreparedAnalysisError, PreprocessInputs,
    PreprocessedAnalysis, ProjectionFailure, ProjectionLimits, ResourceDocument, ResourceLookup,
    ResourceLookupResult,
};
use adocweave::{Analysis, AnalysisInputs, CancellationCheck, Engine, SourceId};

use crate::filesystem::{FilesystemAuthority, FilesystemError, RootAuthority};
use crate::selection::{
    NormalizedSelector, absolute_lexical, identity_path, normalize_selectors, select_targets,
};
use crate::{
    ProjectAnalysis, ProjectConfig, ProjectConfigRequest, ProjectConfigResult,
    ProjectConfigSelection, ProjectConfigSnapshot, ProjectError, ProjectExpandedAnalysis,
    ProjectExpansionError, ProjectLimit, ProjectOutcome, ProjectParseError, ProjectRequest,
    ProjectResourceFailure, ProjectResourceKind, ProjectResourceLimits, ProjectResourceOutcome,
    ProjectResourceResult, ProjectResult, ProjectSourceLocation, ProjectTargetError,
    ProjectTargetResult, ProjectUsage, ProjectWarning, project_authority_error,
    project_config_error, project_expansion_read, project_target_read,
};

pub fn process(request: ProjectRequest, cancellation: &dyn CancellationCheck) -> ProjectOutcome {
    Processor::new(request, cancellation)?.run()
}

pub fn resolve_config(
    request: ProjectConfigRequest,
    cancellation: &dyn CancellationCheck,
) -> Result<ProjectConfigResult, ProjectError> {
    let ProjectConfigRequest {
        authority,
        search_from,
        search_from_is_directory,
        config,
        overrides,
        limits,
    } = request;
    let mut processor = Processor::new(
        ProjectRequest {
            targets: Vec::new(),
            sources: Vec::new(),
            config,
            overrides,
            apply_safe_fixes: false,
            resource_selection: crate::ProjectResourceSelection::default(),
            authority,
            limits,
        },
        cancellation,
    )?;
    let search_from = absolute_lexical(&processor.project_root, &search_from)
        .and_then(|path| processor.filesystem.normalize_path(&path))
        .map_err(project_authority_error)?;
    let resolved = processor.resolve_config_at(&search_from, search_from_is_directory)?;
    if cancellation.is_cancelled() {
        return Err(ProjectError::Cancelled);
    }
    let source_id = resolved.snapshot.as_ref().and_then(|snapshot| {
        processor
            .filesystem_source_id_for_path(&snapshot.path)
            .ok()
            .map(|value| SourceId::new(value.as_str()))
    });
    let config = Arc::new(ProjectConfigSnapshot::from_loaded(
        resolved.snapshot.as_deref(),
        resolved.resolved.as_ref(),
        source_id,
    ));
    let resources = processor.config_observations();
    let (usage, warnings) = processor.finish_usage()?;
    Ok(ProjectConfigResult {
        config,
        resources,
        warnings,
        usage,
    })
}

struct Processor<'request> {
    project_root: PathBuf,
    config_selection: ProjectConfigSelection,
    overrides: crate::ProjectConfigOverrides,
    apply_safe_fixes: bool,
    resource_selection: crate::ProjectResourceSelection,
    limits: crate::ProjectLimits,
    filesystem_limits: crate::ProjectLimits,
    selectors: Vec<NormalizedSelector>,
    no_symlink_targets: BTreeSet<PathBuf>,
    source_targets: BTreeSet<SourceId>,
    authority: FilesystemAuthority,
    identity_roots: Vec<PathBuf>,
    filesystem: FilesystemAuthority,
    fixed: BTreeMap<SourceId, Vec<FixedResource>>,
    memory_sources: BTreeMap<PathBuf, MemorySource>,
    pathless_sources: BTreeMap<SourceId, MemorySource>,
    source_ids_by_path: BTreeMap<PathBuf, SourceId>,
    reserved_source_ids: BTreeMap<SourceId, Option<PathBuf>>,
    inspections: BTreeMap<SourceId, Vec<FixedInspection>>,
    configs: BTreeMap<PathBuf, Arc<LoadedProjectConfig>>,
    resolved_configs: BTreeMap<Option<PathBuf>, Arc<ProjectConfig>>,
    published_configs: BTreeMap<Option<PathBuf>, Arc<ProjectConfigSnapshot>>,
    scope_budgets: BTreeMap<PathBuf, ScopeBudget>,
    inspection_scope_budgets: BTreeMap<PathBuf, ScopeBudget>,
    processing_iterations: u32,
    output_bytes: u64,
    memory_count: usize,
    memory_bytes: u64,
    filesystem_usage: RequestFilesystemUsage,
    directory_operations: u64,
    directory_entries: u64,
    warnings: Vec<ProjectWarning>,
    cancellation: &'request dyn CancellationCheck,
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
                    .any(|observed| same_budget_authority(observed.as_ref(), authority))
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

#[derive(Clone, Debug)]
struct FixedResource {
    source_id: SourceId,
    requested_path: PathBuf,
    path: PathBuf,
    base: Option<PathBuf>,
    no_symlinks: bool,
    authority: Option<RootAuthority>,
    outcome: ProjectResourceOutcome,
    origin: crate::ProjectResourceOrigin,
    observed_bytes: u64,
}

#[derive(Clone, Debug)]
struct MemorySource {
    source_id: SourceId,
    source: Arc<str>,
    exposed_path: Option<PathBuf>,
    base: PathBuf,
}

#[derive(Clone, Debug)]
struct FixedInspection {
    source_id: SourceId,
    requested_path: PathBuf,
    path: PathBuf,
    authority: Option<RootAuthority>,
    outcome: ProjectResourceOutcome,
}

struct TargetAnalysisContext<'target> {
    source_id: &'target SourceId,
    source: &'target Arc<str>,
    config: &'target ProjectConfig,
    allowed_roots: &'target [PathBuf],
    scope: &'target Path,
    bases: &'target mut BTreeMap<String, PathBuf>,
    include_bases: &'target mut BTreeMap<String, PathBuf>,
    lookup_bases: &'target mut BTreeMap<String, PathBuf>,
    resources: &'target mut Vec<ProjectResourceResult>,
    filesystem: &'target mut FilesystemAuthority,
}

struct TargetConfig {
    snapshot: Option<Arc<LoadedProjectConfig>>,
    resolved: Arc<ProjectConfig>,
    resource: Option<ProjectResourceResult>,
}

struct PreparedSource {
    source: Arc<str>,
    replacement: Option<Arc<str>>,
    analysis: Analysis,
}

struct FinishTargetInput {
    source_id: SourceId,
    path: PathBuf,
    replacement_source: Option<Arc<str>>,
    config: Option<Arc<LoadedProjectConfig>>,
    resolved_config: Arc<ProjectConfig>,
    resources: Vec<ProjectResourceResult>,
    outcome: Result<ProjectAnalysis, ProjectTargetError>,
}

struct TargetResultParts {
    source_id: SourceId,
    path: Option<PathBuf>,
    source: Option<Arc<str>>,
    replacement_source: Option<Arc<str>>,
    write: Option<crate::ProjectWriteCapability>,
    config: Arc<ProjectConfigSnapshot>,
    resources: Vec<ProjectResourceResult>,
    outcome: Result<ProjectAnalysis, ProjectTargetError>,
}

enum ExpandedAnalysisFailure {
    Error(ProjectExpansionError),
    Cancelled,
}

enum PreparedSourceFailure {
    Error(ProjectTargetError),
    Cancelled,
}

impl From<ProjectTargetError> for PreparedSourceFailure {
    fn from(error: ProjectTargetError) -> Self {
        Self::Error(error)
    }
}

impl From<ProjectExpansionError> for ExpandedAnalysisFailure {
    fn from(error: ProjectExpansionError) -> Self {
        Self::Error(error)
    }
}

struct FixedReadRequest {
    source_id: SourceId,
    path: PathBuf,
    allowance: Option<ScopeReadAllowance>,
    no_symlinks: bool,
}

struct InspectionRequest<'request> {
    source_id: SourceId,
    path: PathBuf,
    authority: &'request Path,
    base: &'request Path,
    target: &'request str,
}

impl FixedInspection {
    fn result(&self, requested_by: Option<SourceId>) -> ProjectResourceResult {
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
    fn result(
        &self,
        kind: ProjectResourceKind,
        requested_by: Option<SourceId>,
        request_range: Option<adocweave::text::TextRange>,
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

impl<'request> Processor<'request> {
    fn new(
        request: ProjectRequest,
        cancellation: &'request dyn CancellationCheck,
    ) -> Result<Self, ProjectError> {
        if cancellation.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let ProjectRequest {
            targets,
            sources,
            mut config,
            mut overrides,
            apply_safe_fixes,
            resource_selection,
            authority,
            limits,
        } = request;
        let (project_root, mut authority) = authority.into_parts();
        if let ProjectConfigSelection::Explicit(path) = &mut config {
            *path = authority
                .normalize_path(
                    &absolute_lexical(&project_root, path).map_err(project_authority_error)?,
                )
                .map_err(project_authority_error)?;
        }
        if let Some(roots) = &mut overrides.resource_roots {
            for path in roots.iter_mut() {
                *path = absolute_lexical(&project_root, path).map_err(project_authority_error)?;
                *path = authority
                    .normalize_path(path)
                    .map_err(project_authority_error)?;
            }
            roots.sort();
            roots.dedup();
        }
        if let Some(path) = &mut overrides.local_target_project_root {
            *path = absolute_lexical(&project_root, path).map_err(project_authority_error)?;
            *path = authority
                .normalize_path(path)
                .map_err(project_authority_error)?;
        }
        for path in &mut overrides.stylesheet_files {
            *path = absolute_lexical(&project_root, path).map_err(project_authority_error)?;
            *path = authority
                .normalize_path(path)
                .map_err(project_authority_error)?;
        }
        let mut source_ids_by_path = BTreeMap::new();
        let mut memory_sources = BTreeMap::new();
        let mut pathless_sources = BTreeMap::new();
        let mut reserved_source_ids = BTreeMap::new();
        let mut memory_bytes = 0_u64;
        let mut memory_count = 0_usize;
        for source in sources {
            let exposed_path = source
                .path
                .as_ref()
                .map(|path| absolute_lexical(&project_root, path))
                .transpose()
                .map_err(project_authority_error)?
                .map(|path| authority.normalize_path(&path))
                .transpose()
                .map_err(project_authority_error)?;
            let base = absolute_lexical(&project_root, &source.base)
                .and_then(|path| authority.normalize_path(&path))
                .map_err(project_authority_error)?;
            let authority_path = exposed_path.as_ref().unwrap_or(&base);
            authority
                .authority_for_path(authority_path)
                .ok_or_else(|| {
                    project_authority_error(FilesystemError::OutsideRoot(authority_path.clone()))
                })?;
            let source_id = source.source_id.clone();
            if source_id.as_str().starts_with("project:")
                || source_id.as_str().starts_with("authority:")
                || source_id.as_str().starts_with("local-target:")
                || source_id.as_str().starts_with("include-request:")
            {
                return Err(ProjectError::InvalidInput(crate::ProjectInputError::new(
                    "reserved-source-id",
                    "caller source ID uses the filesystem identity namespace",
                )));
            }
            if reserved_source_ids.contains_key(&source_id) {
                return Err(ProjectError::InvalidInput(crate::ProjectInputError::new(
                    "duplicate-source-id",
                    "an in-memory source ID may name only one input",
                )));
            }
            if source.source.len()
                > usize::try_from(limits.resources.max_resource_bytes).unwrap_or(usize::MAX)
            {
                return Err(ProjectError::Limit(ProjectLimit::ResourceBytes {
                    limit: limits.resources.max_resource_bytes,
                }));
            }
            memory_count =
                memory_count
                    .checked_add(1)
                    .ok_or(ProjectError::Limit(ProjectLimit::Files {
                        limit: limits.resources.max_files,
                    }))?;
            if memory_count > limits.resources.max_files {
                return Err(ProjectError::Limit(ProjectLimit::Files {
                    limit: limits.resources.max_files,
                }));
            }
            memory_bytes = memory_bytes
                .checked_add(u64::try_from(source.source.len()).unwrap_or(u64::MAX))
                .ok_or(ProjectError::Limit(ProjectLimit::ReadBytes {
                    limit: limits.resources.max_total_bytes,
                }))?;
            if memory_bytes > limits.resources.max_total_bytes {
                return Err(ProjectError::Limit(ProjectLimit::ReadBytes {
                    limit: limits.resources.max_total_bytes,
                }));
            }
            let memory = MemorySource {
                source_id: source_id.clone(),
                source: source.source,
                exposed_path: exposed_path.clone(),
                base,
            };
            reserved_source_ids.insert(source_id.clone(), exposed_path.clone());
            if let Some(path) = exposed_path {
                if source_ids_by_path.insert(path.clone(), source_id).is_some() {
                    return Err(ProjectError::InvalidInput(crate::ProjectInputError::new(
                        "duplicate-source-path",
                        "more than one in-memory source names the same path",
                    )));
                }
                memory_sources.insert(path, memory);
            } else {
                pathless_sources.insert(source_id, memory);
            }
        }
        let mut source_targets = BTreeSet::new();
        let mut filesystem_targets = Vec::new();
        for target in targets {
            match target {
                crate::ProjectTarget::Source(source_id) => {
                    if !reserved_source_ids.contains_key(&source_id) {
                        return Err(ProjectError::InvalidInput(crate::ProjectInputError::new(
                            "unknown-source-target",
                            format!(
                                "source target is not present in request sources: {}",
                                source_id.as_str()
                            ),
                        )));
                    }
                    source_targets.insert(source_id);
                }
                target => filesystem_targets.push(target),
            }
        }
        let targets = filesystem_targets;
        let filesystem_limits = crate::ProjectLimits {
            resources: ProjectResourceLimits {
                max_files: limits.resources.max_files - memory_count,
                max_total_bytes: limits.resources.max_total_bytes - memory_bytes,
                ..limits.resources
            },
            ..limits
        };
        let project_root = absolute_lexical(
            authority
                .roots()
                .first()
                .map(PathBuf::as_path)
                .unwrap_or_else(|| Path::new("")),
            &project_root,
        )
        .map_err(project_authority_error)?;
        let root_policy = authority
            .authority_for_path(&project_root)
            .cloned()
            .ok_or_else(|| {
                project_authority_error(FilesystemError::OutsideRoot(project_root.clone()))
            })?;
        let project_policy = if root_policy.root() == project_root {
            root_policy
        } else {
            root_policy
                .derive_confined_directory(&project_root)
                .map_err(project_authority_error)?
        };
        let project_root = project_policy.root().to_owned();
        let no_symlink_targets = targets
            .iter()
            .filter_map(|target| match target {
                crate::ProjectTarget::PathNoSymlinks(path) => Some(path),
                _ => None,
            })
            .map(|path| absolute_lexical(&project_root, path).map_err(project_authority_error))
            .map(|path| {
                path.and_then(|path| {
                    authority
                        .normalize_path(&path)
                        .map_err(project_authority_error)
                })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let retained_roots = authority.roots().to_vec();
        let identity_roots = retained_roots.clone();
        authority = authority
            .select(retained_roots)
            .map_err(project_authority_error)?;
        let selectors = normalize_selectors(&project_root, &authority, &targets)?;
        let filesystem = authority.clone();
        Ok(Self {
            project_root,
            config_selection: config,
            overrides,
            apply_safe_fixes,
            resource_selection,
            limits,
            filesystem_limits,
            selectors,
            no_symlink_targets,
            source_targets,
            authority,
            identity_roots,
            filesystem,
            fixed: BTreeMap::new(),
            memory_sources,
            pathless_sources,
            source_ids_by_path,
            reserved_source_ids,
            inspections: BTreeMap::new(),
            configs: BTreeMap::new(),
            resolved_configs: BTreeMap::new(),
            published_configs: BTreeMap::new(),
            scope_budgets: BTreeMap::new(),
            inspection_scope_budgets: BTreeMap::new(),
            processing_iterations: 0,
            output_bytes: 0,
            memory_count,
            memory_bytes,
            filesystem_usage: RequestFilesystemUsage::default(),
            directory_operations: 0,
            directory_entries: 0,
            warnings: Vec::new(),
            cancellation,
        })
    }

    fn run(mut self) -> ProjectOutcome {
        let selectors = self.selectors.clone();
        let paths = select_targets(
            &selectors,
            &self.authority,
            self.filesystem_limits,
            &mut self.warnings,
            &mut self.directory_operations,
            &mut self.directory_entries,
            self.cancellation,
        )?;
        if self.cancellation.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let mut targets = Vec::with_capacity(paths.len());
        let mut processed = BTreeSet::new();
        for path in paths {
            if self.cancellation.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            processed.insert(path.clone());
            let no_symlinks = self.no_symlink_targets.contains(&path);
            let target = self.process_target(path, no_symlinks)?;
            if self.cancellation.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            targets.push(target);
        }
        for source_id in self.source_targets.clone() {
            if self.cancellation.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            if let Some(path) = self
                .reserved_source_ids
                .get(&source_id)
                .cloned()
                .expect("validated source target remains reserved")
            {
                if processed.insert(path.clone()) {
                    let target = self.process_target(path, false)?;
                    if self.cancellation.is_cancelled() {
                        return Err(ProjectError::Cancelled);
                    }
                    targets.push(target);
                }
            } else {
                let source = self
                    .pathless_sources
                    .get(&source_id)
                    .cloned()
                    .expect("validated pathless source target remains available");
                let target = self.process_pathless_target(source)?;
                if self.cancellation.is_cancelled() {
                    return Err(ProjectError::Cancelled);
                }
                targets.push(target);
            }
        }
        let resources = self.config_observations();
        let (usage, warnings) = self.finish_usage()?;
        Ok(ProjectResult {
            targets,
            resources,
            warnings,
            usage,
        })
    }

    fn config_observations(&self) -> Vec<ProjectResourceResult> {
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

    fn finish_usage(self) -> Result<(ProjectUsage, Vec<ProjectWarning>), ProjectError> {
        if let Some(limit) = self.filesystem_usage.limit {
            return Err(ProjectError::Limit(limit));
        }
        let processing_iterations = self.processing_iterations;
        let output_bytes = self.output_bytes;
        let mut warnings = self.warnings;
        warnings.sort_by_key(|warning| format!("{warning:?}"));
        Ok((
            ProjectUsage {
                read_operations: self
                    .filesystem_usage
                    .read_operations
                    .saturating_add(u64::try_from(self.memory_count).unwrap_or(u64::MAX)),
                read_bytes: self
                    .filesystem_usage
                    .read_bytes
                    .saturating_add(self.memory_bytes),
                directory_operations: self.directory_operations,
                directory_entries: self.directory_entries,
                processing_iterations,
                output_bytes,
            },
            warnings,
        ))
    }

    fn process_target(
        &mut self,
        path: PathBuf,
        no_symlinks: bool,
    ) -> Result<ProjectTargetResult, ProjectError> {
        let (config_target, target_is_directory) = self.memory_sources.get(&path).map_or_else(
            || (path.clone(), false),
            |source| {
                source
                    .exposed_path
                    .clone()
                    .map_or_else(|| (source.base.clone(), true), |path| (path, false))
            },
        );
        let TargetConfig {
            snapshot: config_snapshot,
            resolved: config,
            resource: config_resource,
        } = self.resolve_config_at(&config_target, target_is_directory)?;
        let scope = config_snapshot.as_ref().map_or_else(
            || self.project_root.clone(),
            |snapshot| snapshot.path.clone(),
        );
        self.scope_budgets
            .entry(scope.clone())
            .or_insert_with(|| ScopeBudget::new(config.resources.limits));
        let source_id = self.source_id_for_path(&path)?;
        self.inspection_scope_budgets
            .entry(scope.clone())
            .or_insert_with(|| ScopeBudget::new(config.resources.limits));
        let primary_authority = self.filesystem.authority_for_path(&path).cloned();
        let _ = self
            .inspection_scope_budgets
            .get_mut(&scope)
            .expect("a resolved configuration creates its inspection budget")
            .reserve(&source_id, primary_authority.as_ref());
        let primary =
            self.read_document_fixed_scoped(&scope, source_id.clone(), path.clone(), no_symlinks);
        let mut resources = config_resource.into_iter().collect::<Vec<_>>();
        let pathless_primary = self
            .memory_sources
            .get(&path)
            .is_some_and(|source| source.exposed_path.is_none());
        if !pathless_primary {
            resources.push(primary.result(ProjectResourceKind::Primary, None, None));
        }
        let source = match &primary.outcome {
            ProjectResourceOutcome::Loaded { source } => Arc::clone(source),
            ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                return Ok(self.finish_target(FinishTargetInput {
                    source_id,
                    path,
                    replacement_source: None,
                    config: config_snapshot,
                    resolved_config: config,
                    resources,
                    outcome: Err(ProjectTargetError::Incomplete(*limit)),
                }));
            }
            ProjectResourceOutcome::Missing => {
                return Ok(self.finish_target(FinishTargetInput {
                    source_id,
                    path: path.clone(),
                    replacement_source: None,
                    config: config_snapshot,
                    resolved_config: config,
                    resources,
                    outcome: Err(project_target_read(FilesystemError::Missing(path))),
                }));
            }
            ProjectResourceOutcome::Failed(failure) => {
                return Ok(self.finish_target(FinishTargetInput {
                    source_id,
                    path,
                    replacement_source: None,
                    config: config_snapshot,
                    resolved_config: config,
                    resources,
                    outcome: Err(project_target_read(failure.error().clone())),
                }));
            }
            ProjectResourceOutcome::Present => unreachable!("a primary document is read"),
            ProjectResourceOutcome::LoadedOmitted { .. } => {
                unreachable!("fixed primary resources retain acquired content")
            }
        };
        let PreparedSource {
            source,
            replacement: replacement_source,
            analysis: source_analysis,
        } = match self.prepare_source(&source_id, source, config.as_ref()) {
            Ok(prepared) => prepared,
            Err(PreparedSourceFailure::Cancelled) => return Err(ProjectError::Cancelled),
            Err(PreparedSourceFailure::Error(error)) => {
                return Ok(self.finish_target(FinishTargetInput {
                    source_id,
                    path,
                    replacement_source: None,
                    config: config_snapshot,
                    resolved_config: config,
                    resources,
                    outcome: Err(error),
                }));
            }
        };
        let mut bases = BTreeMap::from([(
            source_id.as_str().to_owned(),
            primary
                .base
                .clone()
                .unwrap_or_else(|| self.project_root.clone()),
        )]);
        let mut include_bases = bases.clone();
        let mut lookup_bases = BTreeMap::from([(
            "__adocweave_base__".to_owned(),
            primary
                .base
                .clone()
                .unwrap_or_else(|| self.project_root.clone()),
        )]);
        let base = primary.base.as_deref().unwrap_or(&path);
        let allowed_roots = if config.resources.include {
            include_roots(
                base,
                (self.resource_selection.local_targets && config.local_targets.enabled)
                    .then_some(config.local_targets.project_root.as_deref())
                    .flatten(),
                &config,
            )
        } else {
            vec![base.to_owned()]
        };
        let mut include_filesystem = match self.confined_authority(&allowed_roots) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                return Ok(self.finish_target(FinishTargetInput {
                    source_id,
                    path,
                    replacement_source: None,
                    config: config_snapshot,
                    resolved_config: config,
                    resources,
                    outcome: Ok(ProjectAnalysis {
                        primary: source_analysis,
                        expanded: Err(project_expansion_read(error)),
                    }),
                }));
            }
        };
        let expanded = match self.analyze_target(TargetAnalysisContext {
            source_id: &source_id,
            source: &source,
            config: config.as_ref(),
            allowed_roots: &allowed_roots,
            scope: &scope,
            bases: &mut bases,
            include_bases: &mut include_bases,
            lookup_bases: &mut lookup_bases,
            resources: &mut resources,
            filesystem: &mut include_filesystem,
        }) {
            Ok(analysis) => Ok(analysis),
            Err(ExpandedAnalysisFailure::Error(error)) => Err(error),
            Err(ExpandedAnalysisFailure::Cancelled) => return Err(ProjectError::Cancelled),
        };
        let mut outcome = Ok(ProjectAnalysis {
            primary: source_analysis,
            expanded,
        });
        if self.resource_selection.local_targets
            && let Ok(analysis) = &mut outcome
            && let Ok(expanded) = &mut analysis.expanded
        {
            self.collect_local_targets(
                expanded,
                config.as_ref(),
                &scope,
                &bases,
                &include_bases,
                &mut resources,
            );
        }
        if self.resource_selection.stylesheets {
            self.collect_stylesheets(&source_id, config.as_ref(), &scope, &mut resources);
        }
        if let Some(limit) = resources
            .iter()
            .find_map(|resource| match &resource.outcome {
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit))
                    if resource.kind == ProjectResourceKind::Include =>
                {
                    Some(*limit)
                }
                _ => None,
            })
            && let Ok(analysis) = &mut outcome
        {
            analysis.expanded = Err(ProjectExpansionError::Incomplete(limit));
        }
        Ok(self.finish_target(FinishTargetInput {
            source_id,
            path,
            replacement_source,
            config: config_snapshot,
            resolved_config: config,
            resources,
            outcome,
        }))
    }

    fn process_pathless_target(
        &mut self,
        input: MemorySource,
    ) -> Result<ProjectTargetResult, ProjectError> {
        let TargetConfig {
            snapshot,
            resolved: config,
            resource,
        } = self.resolve_config_at(&input.base, true)?;
        let scope = snapshot.as_ref().map_or_else(
            || self.project_root.clone(),
            |snapshot| snapshot.path.clone(),
        );
        self.scope_budgets
            .entry(scope.clone())
            .or_insert_with(|| ScopeBudget::new(config.resources.limits));
        self.inspection_scope_budgets
            .entry(scope.clone())
            .or_insert_with(|| ScopeBudget::new(config.resources.limits));
        let mut resources = resource.into_iter().collect::<Vec<_>>();
        if let Err(limit) = self
            .reserve_scope(&scope, &input.source_id, None)
            .and_then(|_| {
                self.charge_scope_body(&scope, &input.source_id, None, input.source.len())
            })
        {
            return Ok(self.finish_pathless_target(
                input,
                None,
                snapshot,
                config,
                resources,
                Err(ProjectTargetError::Incomplete(limit)),
            ));
        }
        let PreparedSource {
            source,
            replacement: replacement_source,
            analysis: source_analysis,
        } = match self.prepare_source(&input.source_id, Arc::clone(&input.source), config.as_ref())
        {
            Ok(prepared) => prepared,
            Err(PreparedSourceFailure::Cancelled) => return Err(ProjectError::Cancelled),
            Err(PreparedSourceFailure::Error(error)) => {
                return Ok(self.finish_pathless_target(
                    input,
                    None,
                    snapshot,
                    config,
                    resources,
                    Err(error),
                ));
            }
        };
        let source_base = config
            .local_targets
            .project_root
            .clone()
            .filter(|_| self.resource_selection.local_targets && config.local_targets.enabled)
            .unwrap_or_else(|| input.base.clone());
        let mut bases = BTreeMap::from([(input.source_id.as_str().to_owned(), source_base)]);
        let mut include_bases =
            BTreeMap::from([(input.source_id.as_str().to_owned(), input.base.clone())]);
        let mut lookup_bases =
            BTreeMap::from([("__adocweave_base__".to_owned(), input.base.clone())]);
        let allowed_roots = if config.resources.include {
            include_roots(&input.base, None, &config)
        } else {
            vec![input.base.clone()]
        };
        let mut include_filesystem = match self.confined_authority(&allowed_roots) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                return Ok(self.finish_pathless_target(
                    input,
                    None,
                    snapshot,
                    config,
                    resources,
                    Ok(ProjectAnalysis {
                        primary: source_analysis,
                        expanded: Err(project_expansion_read(error)),
                    }),
                ));
            }
        };
        let expanded = match self.analyze_target(TargetAnalysisContext {
            source_id: &input.source_id,
            source: &source,
            config: config.as_ref(),
            allowed_roots: &allowed_roots,
            scope: &scope,
            bases: &mut bases,
            include_bases: &mut include_bases,
            lookup_bases: &mut lookup_bases,
            resources: &mut resources,
            filesystem: &mut include_filesystem,
        }) {
            Ok(analysis) => Ok(analysis),
            Err(ExpandedAnalysisFailure::Error(error)) => Err(error),
            Err(ExpandedAnalysisFailure::Cancelled) => return Err(ProjectError::Cancelled),
        };
        let mut outcome = Ok(ProjectAnalysis {
            primary: source_analysis,
            expanded,
        });
        if self.resource_selection.local_targets
            && let Ok(analysis) = &mut outcome
            && let Ok(expanded) = &mut analysis.expanded
        {
            self.collect_local_targets(
                expanded,
                config.as_ref(),
                &scope,
                &bases,
                &include_bases,
                &mut resources,
            );
        }
        if self.resource_selection.stylesheets {
            self.collect_stylesheets(&input.source_id, config.as_ref(), &scope, &mut resources);
        }
        if let Some(limit) = resources
            .iter()
            .find_map(|resource| match &resource.outcome {
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit))
                    if resource.kind == ProjectResourceKind::Include =>
                {
                    Some(*limit)
                }
                _ => None,
            })
            && let Ok(analysis) = &mut outcome
        {
            analysis.expanded = Err(ProjectExpansionError::Incomplete(limit));
        }
        Ok(self.finish_pathless_target(
            input,
            replacement_source,
            snapshot,
            config,
            resources,
            outcome,
        ))
    }

    fn finish_pathless_target(
        &mut self,
        input: MemorySource,
        replacement_source: Option<Arc<str>>,
        config: Option<Arc<LoadedProjectConfig>>,
        resolved_config: Arc<ProjectConfig>,
        mut resources: Vec<ProjectResourceResult>,
        mut outcome: Result<ProjectAnalysis, ProjectTargetError>,
    ) -> ProjectTargetResult {
        resources.sort_by(|left, right| {
            left.source_id
                .cmp(&right.source_id)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| resource_kind_order(left.kind).cmp(&resource_kind_order(right.kind)))
                .then_with(|| left.requested_at.cmp(&right.requested_at))
        });
        let primary_bytes = input
            .source
            .len()
            .saturating_add(replacement_source.as_ref().map_or(0, |source| source.len()));
        let returned_bytes = primary_bytes
            .saturating_add(expanded_document_bytes(&outcome))
            .saturating_add(returned_resource_bytes(&resources));
        let total = add_output_bytes(self.output_bytes, returned_bytes);
        if total > u64::from(self.limits.max_output_bytes) {
            let limit = ProjectLimit::OutputBytes {
                limit: self.limits.max_output_bytes,
            };
            omit_loaded_resources(&mut resources, limit);
            let primary_total = add_output_bytes(self.output_bytes, primary_bytes);
            if primary_total <= u64::from(self.limits.max_output_bytes)
                && let Ok(analysis) = &mut outcome
            {
                analysis.expanded = Err(ProjectExpansionError::Incomplete(limit));
                self.output_bytes = primary_total;
            } else {
                outcome = Err(ProjectTargetError::Incomplete(limit));
            }
        } else {
            self.output_bytes = total;
        }
        let key = config.as_ref().map(|snapshot| snapshot.path.clone());
        let published = if let Some(config) = self.published_configs.get(&key) {
            Arc::clone(config)
        } else {
            let source_id = config
                .as_ref()
                .and_then(|snapshot| self.filesystem_source_id_for_path(&snapshot.path).ok())
                .map(|value| SourceId::new(value.as_str()));
            let published = Arc::new(ProjectConfigSnapshot::from_loaded(
                config.as_deref(),
                resolved_config.as_ref(),
                source_id,
            ));
            self.published_configs.insert(key, Arc::clone(&published));
            published
        };
        let source = outcome.is_ok().then_some(input.source);
        let replacement_source = outcome.is_ok().then_some(replacement_source).flatten();
        target_result(TargetResultParts {
            source_id: input.source_id,
            path: None,
            source,
            replacement_source,
            write: None,
            config: published,
            resources,
            outcome,
        })
    }

    fn finish_target(&mut self, input: FinishTargetInput) -> ProjectTargetResult {
        let FinishTargetInput {
            source_id,
            path,
            replacement_source,
            config,
            resolved_config,
            mut resources,
            mut outcome,
        } = input;
        resources.sort_by(|left, right| {
            left.source_id
                .cmp(&right.source_id)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| resource_kind_order(left.kind).cmp(&resource_kind_order(right.kind)))
                .then_with(|| left.requested_at.cmp(&right.requested_at))
        });
        let memory_source = self.memory_sources.get(&path);
        let original_source = memory_source
            .map(|source| Arc::clone(&source.source))
            .or_else(|| {
                resources
                    .iter()
                    .find_map(|resource| match (&resource.kind, &resource.outcome) {
                        (
                            ProjectResourceKind::Primary,
                            ProjectResourceOutcome::Loaded { source },
                        ) => Some(Arc::clone(source)),
                        _ => None,
                    })
            })
            .unwrap_or_else(|| Arc::from(""));
        let primary_bytes = original_source
            .len()
            .saturating_add(replacement_source.as_ref().map_or(0, |source| source.len()));
        let returned_bytes = expanded_document_bytes(&outcome)
            .saturating_add(replacement_source.as_ref().map_or(0, |source| source.len()))
            .saturating_add(returned_resource_bytes(&resources));
        let total = add_output_bytes(self.output_bytes, returned_bytes);
        if total > u64::from(self.limits.max_output_bytes) {
            let limit = ProjectLimit::OutputBytes {
                limit: self.limits.max_output_bytes,
            };
            omit_loaded_resources(&mut resources, limit);
            let primary_total = add_output_bytes(self.output_bytes, primary_bytes);
            if primary_total <= u64::from(self.limits.max_output_bytes)
                && let Ok(analysis) = &mut outcome
            {
                analysis.expanded = Err(ProjectExpansionError::Incomplete(limit));
                self.output_bytes = primary_total;
            } else {
                outcome = Err(ProjectTargetError::Incomplete(limit));
            }
        } else {
            self.output_bytes = total;
        }
        let config_key = config.as_ref().map(|snapshot| snapshot.path.clone());
        let published_config = if let Some(config) = self.published_configs.get(&config_key) {
            Arc::clone(config)
        } else {
            let config_source_id = config
                .as_ref()
                .and_then(|snapshot| self.filesystem_source_id_for_path(&snapshot.path).ok())
                .map(|value| SourceId::new(value.as_str()));
            let published = Arc::new(ProjectConfigSnapshot::from_loaded(
                config.as_deref(),
                resolved_config.as_ref(),
                config_source_id,
            ));
            self.published_configs
                .insert(config_key, Arc::clone(&published));
            published
        };
        let exposed_path =
            memory_source.map_or_else(|| Some(path), |source| source.exposed_path.clone());
        let write = outcome
            .as_ref()
            .is_ok_and(|analysis| analysis.expanded.is_ok())
            .then_some(())
            .and(exposed_path.as_ref())
            .and_then(|path| {
                self.fixed.get(&source_id).and_then(|fixed| {
                    fixed.iter().find_map(|fixed| {
                        (fixed.requested_path == *path
                            && matches!(fixed.outcome, ProjectResourceOutcome::Loaded { .. }))
                        .then(|| fixed.authority.clone())
                        .flatten()
                        .and_then(|policy| {
                            policy.inspect_candidate_no_symlinks(path).ok().map(|_| {
                                crate::ProjectWriteCapability::new(
                                    path.clone(),
                                    policy,
                                    Arc::clone(&original_source),
                                )
                            })
                        })
                    })
                })
            });
        let source = outcome.is_ok().then_some(original_source);
        let replacement_source = outcome.is_ok().then_some(replacement_source).flatten();
        target_result(TargetResultParts {
            source_id,
            path: exposed_path,
            source,
            replacement_source,
            write,
            config: published_config,
            resources,
            outcome,
        })
    }

    fn resolve_config_at(
        &mut self,
        target: &Path,
        target_is_directory: bool,
    ) -> Result<TargetConfig, ProjectError> {
        let path = match self.config_selection.clone() {
            ProjectConfigSelection::Disabled => None,
            ProjectConfigSelection::Explicit(path) => {
                Some(absolute_lexical(&self.project_root, &path).map_err(project_authority_error)?)
            }
            ProjectConfigSelection::Discover => {
                self.discover_config(target, target_is_directory)?
            }
        };
        let Some(path) = path else {
            let resolved = self.resolved_config_for(None);
            return Ok(TargetConfig {
                snapshot: None,
                resolved,
                resource: None,
            });
        };
        let snapshot = self.load_config(path.clone())?;
        let source_id = self.filesystem_source_id_for_path(&path)?;
        let resource = self.fixed.get(&source_id).and_then(|fixed| {
            reusable_resource(fixed, &path, &self.filesystem, true)
                .map(|fixed| fixed.result(ProjectResourceKind::Config, None, None))
        });
        let repair_candidate = resource
            .as_ref()
            .and_then(|resource| resource.observation.clone());
        if !snapshot.path.starts_with(&self.project_root) {
            for configured in snapshot
                .config
                .resources
                .roots
                .iter()
                .filter(|_| snapshot.config.resources.include)
                .chain(
                    snapshot
                        .config
                        .html
                        .stylesheet_files
                        .iter()
                        .filter(|_| self.resource_selection.stylesheets),
                )
                .chain(
                    snapshot
                        .config
                        .local_targets
                        .project_root
                        .iter()
                        .filter(|_| self.resource_selection.local_targets),
                )
            {
                if !configured.starts_with(&self.project_root) {
                    let error =
                        project_authority_error(FilesystemError::OutsideRoot(configured.clone()));
                    return Err(repair_candidate.clone().map_or(error.clone(), |candidate| {
                        error.with_repair_candidate(candidate)
                    }));
                }
            }
        }
        let resolved = self.resolved_config_for(Some(&snapshot));
        Ok(TargetConfig {
            snapshot: Some(snapshot),
            resolved,
            resource,
        })
    }

    fn resolved_config_for(
        &mut self,
        snapshot: Option<&Arc<LoadedProjectConfig>>,
    ) -> Arc<ProjectConfig> {
        let key = snapshot.map(|snapshot| snapshot.path.clone());
        let config =
            snapshot.map_or_else(ProjectConfig::default, |snapshot| snapshot.config.clone());
        self.resolved_config_value(key, Arc::new(config))
    }

    fn resolved_config_value(
        &mut self,
        key: Option<PathBuf>,
        mut config: Arc<ProjectConfig>,
    ) -> Arc<ProjectConfig> {
        if let Some(config) = self.resolved_configs.get(&key) {
            return Arc::clone(config);
        }
        let resolved = Arc::make_mut(&mut config);
        if let Some(include) = self.overrides.include {
            resolved.resources.include = include;
            resolved.preprocess.enable_includes = include;
        }
        if let Some(roots) = &self.overrides.resource_roots {
            resolved.resources.roots.clone_from(roots);
        }
        if let Some(root) = &self.overrides.local_target_project_root {
            resolved.local_targets.enabled = true;
            resolved.local_targets.project_root = Some(root.clone());
        }
        for rule in &self.overrides.enable_lint_rules {
            let current = resolved.analysis.diagnostics.lint.rule(*rule);
            resolved.analysis.diagnostics.lint.set_rule(
                *rule,
                adocweave::output::diagnostics::RuleSettings {
                    enabled: true,
                    ..current
                },
            );
        }
        resolved
            .html
            .stylesheet_files
            .extend(self.overrides.stylesheet_files.iter().cloned());
        self.resolved_configs.insert(key, Arc::clone(&config));
        config
    }

    fn load_config(&mut self, path: PathBuf) -> Result<Arc<LoadedProjectConfig>, ProjectError> {
        if let Some(snapshot) = self.configs.get(&path) {
            return Ok(Arc::clone(snapshot));
        }
        let source_id = self.filesystem_source_id_for_path(&path)?;
        let fixed = self.read_fixed_no_symlinks(source_id, path.clone());
        let ProjectResourceOutcome::Loaded { source } = &fixed.outcome else {
            let candidate = observation_candidate(
                &fixed.outcome,
                &fixed.requested_path,
                fixed.origin,
                crate::ProjectObservationKind::ContentsNoSymlinks,
                fixed.authority.is_some(),
            );
            return match &fixed.outcome {
                ProjectResourceOutcome::Missing => Err(candidate.map_or_else(
                    || project_authority_error(FilesystemError::Missing(path.clone())),
                    |candidate| {
                        project_authority_error(FilesystemError::Missing(path.clone()))
                            .with_repair_candidate(candidate)
                    },
                )),
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                    Err(ProjectError::Limit(*limit))
                }
                ProjectResourceOutcome::Failed(failure) => Err(candidate.map_or_else(
                    || project_authority_error(failure.error().clone()),
                    |candidate| {
                        project_authority_error(failure.error().clone())
                            .with_repair_candidate(candidate)
                    },
                )),
                ProjectResourceOutcome::Present
                | ProjectResourceOutcome::Loaded { .. }
                | ProjectResourceOutcome::LoadedOmitted { .. } => {
                    unreachable!("configuration reads return content, absence or failure")
                }
            };
        };
        let snapshot = Arc::new(
            LoadedProjectConfig::from_utf8_source(fixed.path.clone(), source).map_err(|error| {
                project_config_error(fixed.path.clone(), source.as_bytes(), error)
            })?,
        );
        self.retain_config_output(source.len())?;
        self.configs.insert(path, Arc::clone(&snapshot));
        Ok(snapshot)
    }

    fn retain_config_output(&mut self, bytes: usize) -> Result<(), ProjectError> {
        let total = self
            .output_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        if total > u64::from(self.limits.max_output_bytes) {
            return Err(ProjectError::Limit(ProjectLimit::OutputBytes {
                limit: self.limits.max_output_bytes,
            }));
        }
        self.output_bytes = total;
        Ok(())
    }

    fn discover_config(
        &mut self,
        target: &Path,
        target_is_directory: bool,
    ) -> Result<Option<PathBuf>, ProjectError> {
        if !target.starts_with(&self.project_root) {
            return Ok(None);
        }
        let mut directory = if target_is_directory {
            target.to_owned()
        } else {
            target
                .parent()
                .map(Path::to_owned)
                .unwrap_or_else(|| self.project_root.clone())
        };
        loop {
            if self.cancellation.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let candidate = directory.join(crate::config::FILE_NAME);
            let source_id = self.filesystem_source_id_for_path(&candidate)?;
            let fixed = self.read_fixed_no_symlinks(source_id, candidate.clone());
            match fixed.outcome {
                ProjectResourceOutcome::Loaded { .. } => return Ok(Some(fixed.path)),
                ProjectResourceOutcome::Missing => {}
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                    return Err(ProjectError::Limit(limit));
                }
                ProjectResourceOutcome::Failed(failure) => {
                    return Err(project_authority_error(failure.error().clone()));
                }
                ProjectResourceOutcome::Present => {
                    unreachable!("configuration discovery reads candidates")
                }
                ProjectResourceOutcome::LoadedOmitted { .. } => {
                    unreachable!("fixed configuration resources retain acquired content")
                }
            }
            if directory == self.project_root {
                return Ok(None);
            }
            if !directory.pop() || !directory.starts_with(&self.project_root) {
                return Ok(None);
            }
        }
    }

    fn read_fixed_no_symlinks(&mut self, source_id: SourceId, path: PathBuf) -> FixedResource {
        read_fixed_from(
            &mut self.fixed,
            &self.inspections,
            self.filesystem_limits,
            FixedReadRequest {
                source_id,
                path,
                allowance: None,
                no_symlinks: true,
            },
            &self.filesystem,
            &mut self.filesystem_usage,
        )
    }

    fn analyze_target(
        &mut self,
        context: TargetAnalysisContext<'_>,
    ) -> Result<ProjectExpandedAnalysis, ExpandedAnalysisFailure> {
        let TargetAnalysisContext {
            source_id,
            source,
            config,
            allowed_roots,
            scope,
            bases,
            include_bases,
            lookup_bases,
            resources,
            filesystem,
        } = context;
        let mut preprocess = config.preprocess.clone();
        preprocess.enable_includes = config.resources.include;
        preprocess.source_id = Some(SourceId::new(source_id.as_str()));
        preprocess.base_uri = Some("__adocweave_base__".to_owned());
        let options = EffectiveProcessingOptions::new(config.analysis.clone(), preprocess)
            .map_err(ProjectExpansionError::Options)?;
        let mut lookup = FixedLookup::default();
        self.next_iteration()
            .map_err(ProjectExpansionError::Incomplete)?;
        let mut step = options.preprocess_resumable(source, &lookup, self.cancellation);
        loop {
            match step {
                EffectivePreprocessStep::Complete(prepared) => {
                    let preprocessed = options
                        .analyze_preprocessed(prepared, PreprocessInputs::default())
                        .map_err(map_prepared_error)?;
                    let source_mapping = preprocessed
                        .project_origins_cancellable(ProjectionLimits::default(), self.cancellation)
                        .map_err(map_projection_failure)?;
                    return Ok(ProjectExpandedAnalysis {
                        preprocessed,
                        source_mapping,
                        local_target_diagnostics: Vec::new(),
                    });
                }
                EffectivePreprocessStep::NeedResource(suspended) => {
                    self.next_iteration()
                        .map_err(ProjectExpansionError::Incomplete)?;
                    let request = suspended.request();
                    let target = request.target().to_owned();
                    let requested_by = request
                        .source_id()
                        .map(|id| self.source_id_for_value(id.as_str()))
                        .transpose()
                        .map_err(project_expansion_read)?;
                    let path = requested_by
                        .as_ref()
                        .and_then(|owner| include_bases.get(owner.as_str()))
                        .map(|base| absolute_lexical(base, Path::new(request.authored_target())))
                        .transpose()
                        .map_err(project_expansion_read)?
                        .map_or_else(|| resolve_lookup_path(&target, lookup_bases), Ok)
                        .map_err(project_expansion_read)?;
                    if !allowed_roots.iter().any(|root| path.starts_with(root)) {
                        let owner = requested_by.as_ref().unwrap_or(source_id);
                        let rejected_id = self
                            .include_request_source_id(owner, request.range())
                            .map_err(project_expansion_read)?;
                        let error = FilesystemError::OutsideRoot(path.clone());
                        let fixed = self.fix_failure(
                            rejected_id,
                            path,
                            ProjectResourceFailure::Rejected(
                                crate::ProjectResourceError::from_filesystem(error.clone()),
                            ),
                        );
                        resources.push(fixed.result(
                            ProjectResourceKind::Include,
                            requested_by.clone(),
                            Some(request.range()),
                        ));
                        if self.resource_selection.local_targets && config.local_targets.enabled {
                            let document = ResourceDocument {
                                source_id: SourceId::new(fixed.source_id.as_str()),
                                source: Arc::<str>::from(""),
                            };
                            lookup.entries.insert(
                                target.clone(),
                                ResourceLookupResult::Ready(document.clone()),
                            );
                            let response = request.found(document);
                            step = suspended.resume(response, &lookup, self.cancellation);
                            continue;
                        }
                        return Err(project_expansion_read(error).into());
                    }
                    let include_id = self
                        .source_id_for_path(&path)
                        .map_err(|error| project_expansion_read(project_error_resource(error)))?;
                    if self.resource_selection.local_targets && config.local_targets.enabled {
                        let authority = filesystem.authority_for_path(&path).cloned();
                        let _ = self
                            .inspection_scope_budgets
                            .get_mut(scope)
                            .expect("a resolved configuration creates its inspection budget")
                            .reserve(&include_id, authority.as_ref());
                    }
                    let authored = adocweave::LocalTargetReference::from_include(
                        request.range(),
                        request.range(),
                        request.authored_target(),
                    );
                    if self.resource_selection.local_targets
                        && config.local_targets.enabled
                        && authored.as_ref().is_some_and(|target| {
                            target.syntax == adocweave::LocalTargetSyntax::Unverifiable
                        })
                    {
                        let authored = authored.expect("checked include target");
                        let fixed = self.fix_failure(
                            include_id.clone(),
                            path,
                            ProjectResourceFailure::Rejected(
                                crate::ProjectResourceError::from_filesystem(
                                    FilesystemError::Unverifiable(authored.target),
                                ),
                            ),
                        );
                        resources.push(fixed.result(
                            ProjectResourceKind::Include,
                            requested_by.clone(),
                            Some(request.range()),
                        ));
                        let document = ResourceDocument {
                            source_id: SourceId::new(include_id.as_str()),
                            source: Arc::<str>::from(""),
                        };
                        lookup.entries.insert(
                            target.clone(),
                            ResourceLookupResult::Ready(document.clone()),
                        );
                        let response = request.found(document);
                        step = suspended.resume(response, &lookup, self.cancellation);
                        continue;
                    }
                    let fixed =
                        self.read_fixed_from_scoped(scope, include_id.clone(), path, filesystem);
                    resources.push(fixed.result(
                        ProjectResourceKind::Include,
                        requested_by.clone(),
                        Some(request.range()),
                    ));
                    let response = match &fixed.outcome {
                        ProjectResourceOutcome::Loaded { source } => {
                            if let Some(base) = &fixed.base {
                                bases.insert(include_id.as_str().to_owned(), base.clone());
                                include_bases.insert(include_id.as_str().to_owned(), base.clone());
                                if let Some((lookup_base, _)) = target.rsplit_once('/') {
                                    lookup_bases.insert(lookup_base.to_owned(), base.clone());
                                }
                            }
                            let document = ResourceDocument {
                                source_id: SourceId::new(include_id.as_str()),
                                source: Arc::clone(source),
                            };
                            lookup.entries.insert(
                                target.clone(),
                                ResourceLookupResult::Ready(document.clone()),
                            );
                            request.found(document)
                        }
                        ProjectResourceOutcome::Missing => {
                            if self.resource_selection.local_targets && config.local_targets.enabled
                            {
                                let document = ResourceDocument {
                                    source_id: SourceId::new(include_id.as_str()),
                                    source: Arc::<str>::from(""),
                                };
                                lookup.entries.insert(
                                    target.clone(),
                                    ResourceLookupResult::Ready(document.clone()),
                                );
                                request.found(document)
                            } else {
                                lookup
                                    .entries
                                    .insert(target.clone(), ResourceLookupResult::Missing);
                                request.not_found()
                            }
                        }
                        ProjectResourceOutcome::Failed(failure) => {
                            if self.resource_selection.local_targets && config.local_targets.enabled
                            {
                                let document = ResourceDocument {
                                    source_id: SourceId::new(include_id.as_str()),
                                    source: Arc::<str>::from(""),
                                };
                                lookup.entries.insert(
                                    target.clone(),
                                    ResourceLookupResult::Ready(document.clone()),
                                );
                                request.found(document)
                            } else {
                                let message = failure.to_string();
                                lookup.entries.insert(
                                    target.clone(),
                                    ResourceLookupResult::Failed(message.clone()),
                                );
                                request.load_failed(message)
                            }
                        }
                        ProjectResourceOutcome::Present => unreachable!("an include is read"),
                        ProjectResourceOutcome::LoadedOmitted { .. } => {
                            unreachable!("fixed include resources retain acquired content")
                        }
                    };
                    step = suspended.resume(response, &lookup, self.cancellation);
                }
                EffectivePreprocessStep::Failed(error) => {
                    return Err(ProjectExpansionError::Preprocess(error).into());
                }
                EffectivePreprocessStep::HostError(error) => {
                    return Err(project_expansion_read(FilesystemError::Unverifiable(
                        error.to_string(),
                    ))
                    .into());
                }
                EffectivePreprocessStep::Cancelled => {
                    return Err(ExpandedAnalysisFailure::Cancelled);
                }
                _ => {
                    return Err(project_expansion_read(FilesystemError::Unverifiable(
                        "unknown preprocessing state".to_owned(),
                    ))
                    .into());
                }
            }
        }
    }

    fn prepare_source(
        &self,
        source_id: &SourceId,
        source: Arc<str>,
        config: &ProjectConfig,
    ) -> Result<PreparedSource, PreparedSourceFailure> {
        let core_id = SourceId::new(source_id.as_str());
        let analyze = |source: &str| {
            Engine::new(config.analysis.clone())
                .analyze_with(
                    source,
                    AnalysisInputs {
                        source_id: Some(&core_id),
                        cancellation: Some(self.cancellation),
                    },
                )
                .map_err(|error| {
                    ProjectParseError::from_parse(error)
                        .map_or(PreparedSourceFailure::Cancelled, |error| {
                            PreparedSourceFailure::Error(ProjectTargetError::Parse(error))
                        })
                })
        };
        let analysis = analyze(&source)?;
        if !self.apply_safe_fixes {
            return Ok(PreparedSource {
                source,
                replacement: None,
                analysis,
            });
        }
        let edits = analysis
            .diagnostics()
            .iter()
            .flat_map(|diagnostic| &diagnostic.fixes)
            .filter(|fix| {
                fix.applicability == adocweave::output::diagnostics::Applicability::Always
            })
            .flat_map(|fix| fix.edits().iter().cloned())
            .collect::<Vec<_>>();
        if edits.is_empty() {
            return Ok(PreparedSource {
                source,
                replacement: None,
                analysis,
            });
        }
        let fix = adocweave::output::diagnostics::Fix::new(
            "apply safe fixes",
            adocweave::output::diagnostics::Applicability::Always,
            edits,
        )
        .map_err(|error| ProjectTargetError::EditConflict(error.to_string()))?;
        let mut fixed = source.to_string();
        for edit in fix.edits().iter().rev() {
            fixed.replace_range(
                edit.range.start().to_usize()..edit.range.end().to_usize(),
                &edit.replacement,
            );
        }
        let fixed = Arc::<str>::from(fixed);
        let analysis = analyze(&fixed)?;
        Ok(PreparedSource {
            source: Arc::clone(&fixed),
            replacement: Some(fixed),
            analysis,
        })
    }

    fn collect_stylesheets(
        &mut self,
        requested_by: &SourceId,
        config: &ProjectConfig,
        scope: &Path,
        resources: &mut Vec<ProjectResourceResult>,
    ) {
        let source_limit =
            usize::try_from(adocweave::output::html::StylesheetPolicy::default().max_sources)
                .unwrap_or(usize::MAX);
        if config
            .html
            .stylesheet_files
            .len()
            .saturating_add(config.html.stylesheet_urls.len())
            > source_limit
        {
            return;
        }
        for path in &config.html.stylesheet_files {
            let source_id = match self.filesystem_source_id_for_path(path) {
                Ok(source_id) => source_id,
                Err(ProjectError::Authority(error)) => {
                    self.warnings.push(ProjectWarning::Resource {
                        path: path.clone(),
                        kind: ProjectResourceKind::Stylesheet,
                        failure: ProjectResourceFailure::Rejected(error),
                    });
                    continue;
                }
                Err(error) => {
                    self.warnings.push(ProjectWarning::LocalTargetMapping {
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let fixed = self.read_fixed_scoped(scope, source_id, path.clone());
            resources.push(fixed.result(
                ProjectResourceKind::Stylesheet,
                Some(requested_by.clone()),
                None,
            ));
        }
    }

    fn collect_local_targets(
        &mut self,
        analysis: &mut ProjectExpandedAnalysis,
        config: &ProjectConfig,
        scope: &Path,
        bases: &BTreeMap<String, PathBuf>,
        include_bases: &BTreeMap<String, PathBuf>,
        resources: &mut Vec<ProjectResourceResult>,
    ) {
        if !config.local_targets.enabled {
            return;
        }
        let Some(authority) = config.local_targets.project_root.as_deref() else {
            return;
        };
        let mut local_filesystem = match self.confined_authority(&[authority.to_owned()]) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                self.warnings.push(ProjectWarning::Resource {
                    path: authority.to_owned(),
                    kind: ProjectResourceKind::LocalTarget,
                    failure: ProjectResourceFailure::Rejected(
                        crate::ProjectResourceError::from_filesystem(error),
                    ),
                });
                return;
            }
        };
        let mut candidates = Vec::new();
        for target in &analysis.source_mapping.local_targets {
            let owner = target
                .origins
                .first()
                .and_then(|origin| origin.source_id.as_ref())
                .map_or_else(
                    || {
                        analysis
                            .preprocessed
                            .analysis
                            .source_id()
                            .map(SourceId::as_str)
                    },
                    |source| Some(source.as_str()),
                );
            let Some(owner) = owner else {
                self.warnings.push(ProjectWarning::LocalTargetMapping {
                    message: "local target has no source owner".to_owned(),
                });
                continue;
            };
            let base_by_source = if target.value.kind == adocweave::LocalTargetKind::Include {
                include_bases
            } else {
                bases
            };
            let Some(base) = base_by_source.get(owner).cloned() else {
                self.warnings.push(ProjectWarning::LocalTargetMapping {
                    message: format!("local target owner has no verified base: {owner}"),
                });
                continue;
            };
            candidates.push((owner.to_owned(), base, target.value.clone()));
        }
        let mut seen = BTreeSet::new();
        for (owner, base, target) in candidates {
            let target_kind = target.kind;
            if !seen.insert((
                owner.clone(),
                base.clone(),
                target_kind,
                target.path.clone(),
            )) {
                continue;
            }
            let requested_by = match self.source_id_for_value(&owner) {
                Ok(source_id) => Some(source_id),
                Err(error) => {
                    self.warnings.push(ProjectWarning::Resource {
                        path: base.clone(),
                        kind: ProjectResourceKind::LocalTarget,
                        failure: ProjectResourceFailure::Rejected(
                            crate::ProjectResourceError::from_filesystem(error),
                        ),
                    });
                    None
                }
            };
            let path = match absolute_lexical(&base, Path::new(&target.path)) {
                Ok(path) => path,
                Err(error) => {
                    match self.local_target_source_id(&owner, &base, target_kind, &target.path) {
                        Ok(source_id) => {
                            let fixed = self.fix_failure(
                                source_id,
                                base.join(&target.path),
                                ProjectResourceFailure::Rejected(
                                    crate::ProjectResourceError::from_filesystem(error),
                                ),
                            );
                            resources.push(fixed.result(
                                ProjectResourceKind::LocalTarget,
                                requested_by,
                                None,
                            ));
                        }
                        Err(id_error) => self.warnings.push(ProjectWarning::Resource {
                            path: base.join(&target.path),
                            kind: ProjectResourceKind::LocalTarget,
                            failure: ProjectResourceFailure::Rejected(
                                crate::ProjectResourceError::from_filesystem(id_error),
                            ),
                        }),
                    }
                    continue;
                }
            };
            let source_id = match self.filesystem_source_id_for_path(&path) {
                Ok(source_id) => source_id,
                Err(ProjectError::Authority(error)) => {
                    match self.local_target_source_id(&owner, &base, target_kind, &target.path) {
                        Ok(source_id) => {
                            let fixed = self.fix_failure(
                                source_id,
                                path,
                                ProjectResourceFailure::Rejected(error),
                            );
                            resources.push(fixed.result(
                                ProjectResourceKind::LocalTarget,
                                requested_by,
                                None,
                            ));
                        }
                        Err(id_error) => self.warnings.push(ProjectWarning::Resource {
                            path,
                            kind: ProjectResourceKind::LocalTarget,
                            failure: ProjectResourceFailure::Rejected(
                                crate::ProjectResourceError::from_filesystem(id_error),
                            ),
                        }),
                    }
                    continue;
                }
                Err(error) => {
                    self.warnings.push(ProjectWarning::LocalTargetMapping {
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let result = if target.syntax == adocweave::LocalTargetSyntax::Unverifiable {
                self.fix_failure(
                    source_id,
                    path,
                    ProjectResourceFailure::Rejected(crate::ProjectResourceError::from_filesystem(
                        FilesystemError::Unverifiable(target.target),
                    )),
                )
                .result(ProjectResourceKind::LocalTarget, requested_by, None)
            } else {
                self.inspect_fixed_in(
                    scope,
                    InspectionRequest {
                        source_id,
                        path,
                        authority,
                        base: &base,
                        target: &target.path,
                    },
                    &mut local_filesystem,
                )
                .result(requested_by)
            };
            resources.push(result);
        }
        analysis.local_target_diagnostics = local_target_diagnostics(
            &analysis.source_mapping,
            &analysis.preprocessed,
            bases,
            include_bases,
            resources,
        );
    }

    fn read_fixed(
        &mut self,
        source_id: SourceId,
        path: PathBuf,
        allowance: ScopeReadAllowance,
    ) -> FixedResource {
        read_fixed_from(
            &mut self.fixed,
            &self.inspections,
            self.filesystem_limits,
            FixedReadRequest {
                source_id,
                path,
                allowance: Some(allowance),
                no_symlinks: false,
            },
            &self.filesystem,
            &mut self.filesystem_usage,
        )
    }

    fn read_fixed_scoped(
        &mut self,
        scope: &Path,
        source_id: SourceId,
        path: PathBuf,
    ) -> FixedResource {
        let authority = self.filesystem.authority_for_path(&path).cloned();
        if let Err(limit) = self.reserve_scope(scope, &source_id, authority.as_ref()) {
            return limited_resource(source_id, path, false, authority, limit);
        }
        let allowance = match self.scope_read_allowance(scope, &source_id, authority.as_ref()) {
            Ok(allowance) => allowance,
            Err(limit) => return limited_resource(source_id, path, false, authority, limit),
        };
        let fixed = self.read_fixed(source_id.clone(), path.clone(), allowance);
        self.fix_scope_read_limit(scope, &source_id, authority.as_ref(), &fixed, allowance);
        self.apply_body_budget(scope, fixed, source_id, path)
    }

    fn read_document_fixed_scoped(
        &mut self,
        scope: &Path,
        source_id: SourceId,
        path: PathBuf,
        no_symlinks: bool,
    ) -> FixedResource {
        let authority = self.filesystem.authority_for_path(&path).cloned();
        if let Err(limit) = self.reserve_scope(scope, &source_id, authority.as_ref()) {
            return limited_resource(source_id, path, false, authority, limit);
        }
        if let Some(source) = self.memory_sources.get(&path).cloned() {
            let fixed = memory_resource(source, path.clone(), false, authority);
            return self.apply_body_budget(scope, fixed, source_id, path);
        }
        let allowance = match self.scope_read_allowance(scope, &source_id, authority.as_ref()) {
            Ok(allowance) => allowance,
            Err(limit) => return limited_resource(source_id, path, false, authority, limit),
        };
        let fixed = read_fixed_from(
            &mut self.fixed,
            &self.inspections,
            self.filesystem_limits,
            FixedReadRequest {
                source_id: source_id.clone(),
                path: path.clone(),
                allowance: Some(allowance),
                no_symlinks,
            },
            &self.filesystem,
            &mut self.filesystem_usage,
        );
        self.fix_scope_read_limit(scope, &source_id, authority.as_ref(), &fixed, allowance);
        self.apply_body_budget(scope, fixed, source_id, path)
    }

    fn read_fixed_from_scoped(
        &mut self,
        scope: &Path,
        source_id: SourceId,
        path: PathBuf,
        filesystem: &mut FilesystemAuthority,
    ) -> FixedResource {
        let authority = filesystem.authority_for_path(&path).cloned();
        if let Err(limit) = self.reserve_scope(scope, &source_id, authority.as_ref()) {
            return limited_resource(source_id, path, false, authority, limit);
        }
        if let Some(source) = self.memory_sources.get(&path).cloned() {
            let fixed = memory_resource(source, path.clone(), false, authority);
            return self.apply_body_budget(scope, fixed, source_id, path);
        }
        let allowance = match self.scope_read_allowance(scope, &source_id, authority.as_ref()) {
            Ok(allowance) => allowance,
            Err(limit) => return limited_resource(source_id, path, false, authority, limit),
        };
        let fixed = read_fixed_from(
            &mut self.fixed,
            &self.inspections,
            self.filesystem_limits,
            FixedReadRequest {
                source_id: source_id.clone(),
                path: path.clone(),
                allowance: Some(allowance),
                no_symlinks: false,
            },
            filesystem,
            &mut self.filesystem_usage,
        );
        self.fix_scope_read_limit(scope, &source_id, authority.as_ref(), &fixed, allowance);
        self.apply_body_budget(scope, fixed, source_id, path)
    }

    fn apply_body_budget(
        &mut self,
        scope: &Path,
        fixed: FixedResource,
        source_id: SourceId,
        requested_path: PathBuf,
    ) -> FixedResource {
        let ProjectResourceOutcome::Loaded { source } = &fixed.outcome else {
            return fixed;
        };
        if let Err(limit) =
            self.charge_scope_body(scope, &source_id, fixed.authority.as_ref(), source.len())
        {
            return limited_resource(
                source_id,
                requested_path,
                fixed.no_symlinks,
                fixed.authority,
                limit,
            );
        }
        fixed
    }

    fn reserve_scope(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
    ) -> Result<(), ProjectLimit> {
        self.scope_budgets
            .get_mut(scope)
            .expect("a resolved configuration creates its scope budget")
            .reserve(source_id, authority)
    }

    fn charge_scope_body(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        bytes: usize,
    ) -> Result<(), ProjectLimit> {
        self.scope_budgets
            .get_mut(scope)
            .expect("a resolved configuration creates its scope budget")
            .charge_body(source_id, authority, bytes)
    }

    fn scope_read_allowance(
        &self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
    ) -> Result<ScopeReadAllowance, ProjectLimit> {
        let request_remaining_bytes = self
            .limits
            .resources
            .max_total_bytes
            .saturating_sub(self.filesystem_usage.read_bytes);
        self.scope_budgets
            .get(scope)
            .expect("a resolved configuration creates its scope budget")
            .read_allowance(
                source_id,
                authority,
                self.filesystem_limits.resources,
                request_remaining_bytes,
            )
    }

    fn fix_scope_read_limit(
        &mut self,
        scope: &Path,
        source_id: &SourceId,
        authority: Option<&RootAuthority>,
        fixed: &FixedResource,
        allowance: ScopeReadAllowance,
    ) {
        if fixed.observed_bytes > 0
            && let Err(limit) = self
                .scope_budgets
                .get_mut(scope)
                .expect("a resolved configuration creates its scope budget")
                .charge_observed_bytes(source_id, authority, fixed.observed_bytes)
        {
            self.scope_budgets
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
            self.scope_budgets
                .get_mut(scope)
                .expect("a resolved configuration creates its scope budget")
                .fix_limit(source_id, authority, allowance.limit);
        }
    }

    fn inspect_fixed_in(
        &mut self,
        scope: &Path,
        request: InspectionRequest<'_>,
        filesystem: &mut FilesystemAuthority,
    ) -> FixedInspection {
        let InspectionRequest {
            source_id,
            path,
            authority,
            base,
            target,
        } = request;
        let budget_authority = filesystem.authority_for_path(&path).cloned();
        if let Err(limit) = self
            .inspection_scope_budgets
            .get_mut(scope)
            .expect("a resolved configuration creates its inspection budget")
            .reserve(&source_id, budget_authority.as_ref())
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
        if let Err(limit) = self.filesystem_usage.reserve(
            &source_id,
            acquired_authority.as_ref(),
            self.filesystem_limits.resources.max_files,
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
                ProjectResourceOutcome::Failed(classify_resource_failure(error, self.limits)),
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

    fn fix_failure(
        &mut self,
        source_id: SourceId,
        path: PathBuf,
        failure: ProjectResourceFailure,
    ) -> FixedResource {
        FixedResource {
            source_id: source_id.clone(),
            requested_path: path.clone(),
            path,
            base: None,
            no_symlinks: false,
            authority: None,
            outcome: ProjectResourceOutcome::Failed(failure),
            origin: crate::ProjectResourceOrigin::Filesystem,
            observed_bytes: 0,
        }
    }

    fn next_iteration(&mut self) -> Result<(), ProjectLimit> {
        if self.processing_iterations >= self.limits.max_processing_iterations {
            return Err(ProjectLimit::ProcessingIterations {
                limit: self.limits.max_processing_iterations,
            });
        }
        self.processing_iterations += 1;
        Ok(())
    }

    fn confined_authority(
        &mut self,
        roots: &[PathBuf],
    ) -> Result<FilesystemAuthority, FilesystemError> {
        derive_confined_authority(&mut self.authority, roots)
    }

    fn source_id_for_path(&self, path: &Path) -> Result<SourceId, ProjectError> {
        if let Some(source_id) = self.source_ids_by_path.get(path) {
            return Ok(source_id.clone());
        }
        self.filesystem_source_id_for_path(path)
    }

    fn filesystem_source_id_for_path(&self, path: &Path) -> Result<SourceId, ProjectError> {
        let path = self
            .filesystem
            .normalize_path(path)
            .map_err(project_authority_error)?;
        let value = if path.starts_with(&self.project_root) {
            format!("project:{}", identity_path(&self.project_root, &path))
        } else {
            let (index, root) = self
                .identity_roots
                .iter()
                .enumerate()
                .filter(|(_, root)| path.starts_with(root))
                .max_by_key(|(_, root)| root.components().count())
                .ok_or_else(|| {
                    project_authority_error(FilesystemError::OutsideRoot(path.to_owned()))
                })?;
            format!("authority:{index}:{}", identity_path(root, &path))
        };
        let source_id = self
            .source_id_for_value(&value)
            .map_err(project_authority_error)?;
        if let Some(reserved_path) = self.reserved_source_ids.get(&source_id)
            && reserved_path.as_deref() != Some(path.as_path())
        {
            return Err(ProjectError::InvalidInput(crate::ProjectInputError::new(
                "source-id-collision",
                format!(
                    "generated source ID collides with caller input: {}",
                    source_id.as_str()
                ),
            )));
        }
        Ok(source_id)
    }

    fn source_id_for_value(&self, value: &str) -> Result<SourceId, FilesystemError> {
        Ok(SourceId::new(if value.is_empty() { "." } else { value }))
    }

    fn local_target_source_id(
        &self,
        owner: &str,
        base: &Path,
        kind: adocweave::LocalTargetKind,
        target: &str,
    ) -> Result<SourceId, FilesystemError> {
        self.source_id_for_value(&format!(
            "local-target:{owner}:{}:{kind:?}:{target}",
            identity_path(&self.project_root, base)
        ))
    }

    fn include_request_source_id(
        &self,
        owner: &SourceId,
        range: adocweave::text::TextRange,
    ) -> Result<SourceId, FilesystemError> {
        self.source_id_for_value(&format!(
            "include-request:{}:{}:{}",
            owner.as_str(),
            range.start().to_u32(),
            range.end().to_u32()
        ))
    }
}

fn derive_confined_authority(
    authority: &mut FilesystemAuthority,
    roots: &[PathBuf],
) -> Result<FilesystemAuthority, FilesystemError> {
    let mut staged = authority.clone();
    let mut selected = Vec::new();
    for root in roots {
        let anchor = staged
            .authority_for_path(root)
            .map(|policy| policy.root().to_owned())
            .ok_or_else(|| FilesystemError::OutsideRoot(root.clone()))?;
        let access = if anchor == *root {
            staged.select([anchor])?
        } else {
            staged.derive(&anchor, [root.clone()])?
        };
        selected.extend(access.roots().iter().cloned());
    }
    selected.sort();
    selected.dedup();
    let selected = staged.select(selected)?;
    *authority = staged;
    Ok(selected)
}

fn local_target_diagnostics(
    projection: &adocweave::preprocess::AnalysisProjection,
    preprocessed: &PreprocessedAnalysis,
    bases: &BTreeMap<String, PathBuf>,
    include_bases: &BTreeMap<String, PathBuf>,
    resources: &[ProjectResourceResult],
) -> Vec<crate::ProjectLocalTargetDiagnostic> {
    let fallback_owner = preprocessed.analysis.source_id().map(SourceId::as_str);
    let mut diagnostics = Vec::new();
    for target in &projection.local_targets {
        for origin in &target.target_origins {
            let Some(owner) = origin
                .source_id
                .as_ref()
                .map(SourceId::as_str)
                .or(fallback_owner)
            else {
                continue;
            };
            let base_by_source = if target.value.kind == adocweave::LocalTargetKind::Include {
                include_bases
            } else {
                bases
            };
            let Some(base) = base_by_source.get(owner) else {
                continue;
            };
            let requested_path = absolute_lexical(base, Path::new(&target.value.path))
                .unwrap_or_else(|_| base.join(&target.value.path));
            let Some(resource) = resources.iter().find(|resource| {
                resource.kind == ProjectResourceKind::LocalTarget
                    && resource.requested_path == requested_path
                    && resource
                        .requested_at
                        .as_ref()
                        .map(|location| location.source_id.as_str())
                        == Some(owner)
            }) else {
                continue;
            };
            let Some(error) = local_target_error(&resource.outcome, &resource.path) else {
                continue;
            };
            let range = origin.range.text_range();
            let optional = target.value.kind == adocweave::LocalTargetKind::Include
                && projection.directives.iter().any(|directive| {
                    directive.kind == adocweave::preprocess::DirectiveKind::Include
                        && directive.source_id.as_ref().map(SourceId::as_str) == Some(owner)
                        && directive.target_range == range
                        && directive.optional
                });
            if optional && matches!(error, FilesystemError::Missing(_)) {
                continue;
            }
            let code = error.diagnostic_code();
            diagnostics.push(crate::ProjectLocalTargetDiagnostic {
                diagnostic: adocweave::output::diagnostics::Diagnostic {
                    id: adocweave::output::diagnostics::DiagnosticId::new(format!(
                        "{code}@{owner}:{}:{}",
                        range.start().to_u32(),
                        range.end().to_u32()
                    )),
                    code: adocweave::output::diagnostics::DiagnosticCode::new(code),
                    severity: adocweave::output::diagnostics::Severity::Error,
                    range,
                    message: local_target_message(&error).to_owned(),
                    related: Vec::new(),
                    fixes: Vec::new(),
                },
                source_id: SourceId::new(owner),
                target: target.value.target.clone(),
            });
        }
    }
    for directive in &projection.directives {
        if directive.kind != adocweave::preprocess::DirectiveKind::Include {
            continue;
        }
        let Some(target) = directive.local_target() else {
            continue;
        };
        let Some(owner) = directive
            .source_id
            .as_ref()
            .map(SourceId::as_str)
            .or(fallback_owner)
        else {
            continue;
        };
        let Some(resource) = resources.iter().find(|resource| {
            resource.kind == ProjectResourceKind::Include
                && resource
                    .requested_at
                    .as_ref()
                    .map(|location| location.source_id.as_str())
                    == Some(owner)
                && directive.resource_source_id.as_ref() == Some(&resource.source_id)
        }) else {
            continue;
        };
        let Some(error) = local_target_error(&resource.outcome, &resource.path) else {
            continue;
        };
        if directive.optional && matches!(error, FilesystemError::Missing(_)) {
            continue;
        }
        let range = directive.target_range;
        if diagnostics.iter().any(|diagnostic| {
            diagnostic.source_id.as_str() == owner && diagnostic.diagnostic.range == range
        }) {
            continue;
        }
        let code = error.diagnostic_code();
        diagnostics.push(crate::ProjectLocalTargetDiagnostic {
            diagnostic: adocweave::output::diagnostics::Diagnostic {
                id: adocweave::output::diagnostics::DiagnosticId::new(format!(
                    "{code}@{owner}:{}:{}",
                    range.start().to_u32(),
                    range.end().to_u32()
                )),
                code: adocweave::output::diagnostics::DiagnosticCode::new(code),
                severity: adocweave::output::diagnostics::Severity::Error,
                range,
                message: local_target_message(&error).to_owned(),
                related: Vec::new(),
                fixes: Vec::new(),
            },
            source_id: SourceId::new(owner),
            target: target.target,
        });
    }
    diagnostics.sort_by(|left, right| {
        left.source_id
            .cmp(&right.source_id)
            .then_with(|| {
                left.diagnostic
                    .range
                    .start()
                    .cmp(&right.diagnostic.range.start())
            })
            .then_with(|| {
                left.diagnostic
                    .range
                    .end()
                    .cmp(&right.diagnostic.range.end())
            })
            .then_with(|| left.diagnostic.code.cmp(&right.diagnostic.code))
            .then_with(|| left.target.cmp(&right.target))
    });
    diagnostics
}

fn local_target_error(outcome: &ProjectResourceOutcome, path: &Path) -> Option<FilesystemError> {
    match outcome {
        ProjectResourceOutcome::Present
        | ProjectResourceOutcome::Loaded { .. }
        | ProjectResourceOutcome::LoadedOmitted { .. } => None,
        ProjectResourceOutcome::Missing => Some(FilesystemError::Missing(path.to_owned())),
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => Some(match limit {
            ProjectLimit::Files { limit } => FilesystemError::LimitExceeded { limit: *limit },
            ProjectLimit::ResourceBytes { .. }
            | ProjectLimit::ReadBytes { .. }
            | ProjectLimit::DirectoryEntries { .. }
            | ProjectLimit::ProcessingIterations { .. }
            | ProjectLimit::OutputBytes { .. } => FilesystemError::ReadLimitExceeded,
        }),
        ProjectResourceOutcome::Failed(
            ProjectResourceFailure::Unreadable(error) | ProjectResourceFailure::Rejected(error),
        ) => Some(error.filesystem().clone()),
    }
}

const fn local_target_message(error: &FilesystemError) -> &'static str {
    match error {
        FilesystemError::Missing(_) => "local target does not exist",
        FilesystemError::OutsideRoot(_) => "local target is outside the project root",
        FilesystemError::PathNotAbsolute(_) => "local target path is invalid",
        FilesystemError::NotFile(_) | FilesystemError::NotDirectory(_) => {
            "local target is not a regular file"
        }
        FilesystemError::PermissionDenied(_) => "local target cannot be read",
        FilesystemError::LimitExceeded { .. } => "local target inspection limit exceeded",
        FilesystemError::InvalidUtf8(_)
        | FilesystemError::Inspect { .. }
        | FilesystemError::Unverifiable(_)
        | FilesystemError::ResourceTooLarge(_)
        | FilesystemError::ReadLimitExceeded => "local target cannot be verified",
    }
}

fn read_fixed_from(
    fixed_resources: &mut BTreeMap<SourceId, Vec<FixedResource>>,
    fixed_inspections: &BTreeMap<SourceId, Vec<FixedInspection>>,
    limits: crate::ProjectLimits,
    request: FixedReadRequest,
    filesystem: &FilesystemAuthority,
    usage: &mut RequestFilesystemUsage,
) -> FixedResource {
    let FixedReadRequest {
        source_id,
        path,
        allowance,
        no_symlinks,
    } = request;
    if let Some(fixed) = fixed_resources
        .get(&source_id)
        .and_then(|fixed| reusable_resource(fixed, &path, filesystem, no_symlinks))
    {
        return fixed;
    }
    let authority = filesystem.authority_for_path(&path).cloned();
    if let Err(limit) = usage.reserve(&source_id, authority.as_ref(), limits.resources.max_files) {
        return limited_resource(source_id, path, no_symlinks, authority, limit);
    }
    if let Ok(canonical) = filesystem.inspect(&path, no_symlinks)
        && let Some(source) = fixed_resources.values().flatten().find_map(|fixed| {
            (fixed.path == canonical
                && (!no_symlinks || fixed.no_symlinks)
                && same_budget_authority(
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
        fixed_resources
            .entry(source_id)
            .or_default()
            .push(fixed.clone());
        return fixed;
    }
    if let Some(inspection) = fixed_inspections
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
        fixed_resources
            .entry(source_id)
            .or_default()
            .push(fixed.clone());
        return fixed;
    }

    let requested_path = path.clone();
    let (max_bytes, exhausted_limit) = allowance.map_or_else(
        || {
            let remaining = limits
                .resources
                .max_total_bytes
                .saturating_sub(usage.read_bytes);
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
            usage.limit = Some(exhausted_limit);
        }
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(exhausted_limit))
    } else {
        let read = filesystem.read_utf8(&path, max_bytes, no_symlinks);
        observed_bytes = read.observed_bytes;
        usage.read_bytes = usage.read_bytes.saturating_add(read.observed_bytes);
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
                    usage.limit = Some(exhausted_limit);
                }
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(exhausted_limit))
            }
            Err(error) => ProjectResourceOutcome::Failed(classify_resource_failure(error, limits)),
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
        fixed_resources
            .entry(source_id)
            .or_default()
            .push(fixed.clone());
        observe_fixed(&fixed);
    }
    fixed
}

fn reusable_resource(
    fixed: &[FixedResource],
    requested_path: &Path,
    filesystem: &FilesystemAuthority,
    no_symlinks: bool,
) -> Option<FixedResource> {
    fixed.iter().find_map(|fixed| {
        (validate_cached_authority(
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
        validate_cached_authority(
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

fn resolve_lookup_path(
    target: &str,
    lookup_bases: &BTreeMap<String, PathBuf>,
) -> Result<PathBuf, FilesystemError> {
    let (key, base) = lookup_bases
        .iter()
        .filter(|(key, _)| target == key.as_str() || target.starts_with(&format!("{key}/")))
        .max_by_key(|(key, _)| key.len())
        .ok_or_else(|| {
            FilesystemError::Unverifiable(
                "include target has no verified filesystem base".to_owned(),
            )
        })?;
    let relative = target
        .strip_prefix(key)
        .and_then(|value| value.strip_prefix('/'))
        .unwrap_or("");
    absolute_lexical(base, Path::new(relative))
}

#[cfg(test)]
fn cached_for_authority(
    fixed: &FixedResource,
    requested_path: &Path,
    filesystem: &FilesystemAuthority,
    require_no_symlinks: bool,
) -> FixedResource {
    let access = validate_cached_authority(
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
    );
    if access.is_ok() && (!require_no_symlinks || fixed.no_symlinks) {
        return fixed.clone();
    }
    let mut rejected = fixed.clone();
    rejected.requested_path = requested_path.to_owned();
    rejected.path = requested_path.to_owned();
    let error = if let Err(error) = access {
        error
    } else {
        FilesystemError::Unverifiable(
            "resource was not acquired with symbolic links forbidden".to_owned(),
        )
    };
    rejected.outcome = ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(
        crate::ProjectResourceError::from_filesystem(error),
    ));
    rejected
}

#[cfg(test)]
fn cached_inspection_for_authority(
    fixed: &FixedInspection,
    requested_path: &Path,
    filesystem: &FilesystemAuthority,
) -> FixedInspection {
    let access = validate_cached_authority(
        requested_path,
        &fixed.requested_path,
        &fixed.path,
        matches!(fixed.outcome, ProjectResourceOutcome::Present),
        fixed.authority.as_ref(),
        filesystem,
    );
    if access.is_ok() {
        return fixed.clone();
    }
    let mut rejected = fixed.clone();
    rejected.requested_path = requested_path.to_owned();
    rejected.path = requested_path.to_owned();
    rejected.outcome = ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(
        crate::ProjectResourceError::from_filesystem(
            access.expect_err("failed cache authority is retained"),
        ),
    ));
    rejected
}

fn validate_cached_authority(
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

fn expanded_document_bytes(outcome: &Result<ProjectAnalysis, ProjectTargetError>) -> usize {
    outcome
        .as_ref()
        .ok()
        .and_then(|analysis| analysis.expanded.as_ref().ok())
        .map_or(0, |expanded| expanded.preprocessed.document.source.len())
}

fn returned_resource_bytes(resources: &[ProjectResourceResult]) -> usize {
    resources
        .iter()
        .map(|resource| match &resource.outcome {
            ProjectResourceOutcome::Loaded { source }
                if resource.kind != ProjectResourceKind::Config =>
            {
                source.len()
            }
            _ => 0,
        })
        .sum()
}

fn add_output_bytes(current: u64, added: usize) -> u64 {
    current.saturating_add(u64::try_from(added).unwrap_or(u64::MAX))
}

fn omit_loaded_resources(resources: &mut [ProjectResourceResult], limit: ProjectLimit) {
    for resource in resources {
        if matches!(resource.outcome, ProjectResourceOutcome::Loaded { .. }) {
            resource.outcome = ProjectResourceOutcome::LoadedOmitted { limit };
        }
    }
}

fn target_result(parts: TargetResultParts) -> ProjectTargetResult {
    let TargetResultParts {
        source_id,
        path,
        source,
        replacement_source,
        write,
        config,
        resources,
        outcome,
    } = parts;
    ProjectTargetResult {
        source_id: SourceId::new(source_id.as_str()),
        path,
        source,
        replacement_source,
        write,
        config,
        resources,
        analysis: outcome,
    }
}

fn observation_candidate(
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

fn include_roots(base: &Path, project_root: Option<&Path>, config: &ProjectConfig) -> Vec<PathBuf> {
    let mut roots = if config.resources.roots.is_empty() {
        vec![project_root.unwrap_or(base).to_owned()]
    } else {
        config.resources.roots.clone()
    };
    roots.sort();
    roots.dedup();
    roots
}

#[derive(Default)]
struct FixedLookup {
    entries: BTreeMap<String, ResourceLookupResult>,
}

impl ResourceLookup for FixedLookup {
    fn lookup(&self, target: &str) -> ResourceLookupResult {
        self.entries
            .get(target)
            .cloned()
            .unwrap_or(ResourceLookupResult::Deferred)
    }
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
struct ScopeReadAllowance {
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
            .find(|observation| same_budget_authority(observation.authority.as_ref(), authority))
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
            .find(|observation| same_budget_authority(observation.authority.as_ref(), authority))
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
            .find(|observation| same_budget_authority(observation.authority.as_ref(), authority))
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
                observations.iter_mut().find(|observation| {
                    same_budget_authority(observation.authority.as_ref(), authority)
                })
            })
            .expect("a fixed limit follows a reserved observation");
        observation.limit = Some(limit);
    }
}

fn same_budget_authority(left: Option<&RootAuthority>, right: Option<&RootAuthority>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.has_same_authority(right),
        (None, None) => true,
        _ => false,
    }
}

fn limited_resource(
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

fn memory_resource(
    source: MemorySource,
    path: PathBuf,
    no_symlinks: bool,
    authority: Option<RootAuthority>,
) -> FixedResource {
    let outcome = if authority.is_some() {
        ProjectResourceOutcome::Loaded {
            source: source.source,
        }
    } else {
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(
            crate::ProjectResourceError::from_filesystem(FilesystemError::OutsideRoot(
                path.clone(),
            )),
        ))
    };
    FixedResource {
        source_id: source.source_id,
        requested_path: path.clone(),
        path: path.clone(),
        base: Some(source.base),
        no_symlinks,
        authority,
        outcome,
        origin: crate::ProjectResourceOrigin::Input,
        observed_bytes: 0,
    }
}

fn map_prepared_error(error: PreparedAnalysisError) -> ExpandedAnalysisFailure {
    match error {
        PreparedAnalysisError::ContractMismatch => {
            ExpandedAnalysisFailure::Error(project_expansion_read(FilesystemError::Unverifiable(
                "processing contract mismatch".to_owned(),
            )))
        }
        PreparedAnalysisError::Parse(adocweave::ParseError::Cancelled)
        | PreparedAnalysisError::Cancelled => ExpandedAnalysisFailure::Cancelled,
        PreparedAnalysisError::Parse(error) => {
            ExpandedAnalysisFailure::Error(ProjectExpansionError::Parse(
                ProjectParseError::from_parse(error)
                    .expect("cancelled parsing is handled separately"),
            ))
        }
    }
}

fn map_projection_failure(error: ProjectionFailure) -> ExpandedAnalysisFailure {
    match error {
        ProjectionFailure::LimitExceeded(error) => {
            ExpandedAnalysisFailure::Error(ProjectExpansionError::Projection(error))
        }
        ProjectionFailure::Cancelled => ExpandedAnalysisFailure::Cancelled,
    }
}

fn project_error_resource(error: ProjectError) -> FilesystemError {
    match error {
        ProjectError::Authority(error) => error.filesystem().clone(),
        error => FilesystemError::Unverifiable(error.to_string()),
    }
}

fn classify_resource_failure(
    error: FilesystemError,
    limits: crate::ProjectLimits,
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

impl ProjectResourceFailure {
    fn error(&self) -> &FilesystemError {
        match self {
            Self::Unreadable(error) | Self::Rejected(error) => error.filesystem(),
            Self::Limit(_) => unreachable!("limits are handled before reading their error"),
        }
    }
}

const fn resource_kind_order(kind: ProjectResourceKind) -> u8 {
    match kind {
        ProjectResourceKind::Config => 0,
        ProjectResourceKind::Primary => 1,
        ProjectResourceKind::Include => 2,
        ProjectResourceKind::Stylesheet => 3,
        ProjectResourceKind::LocalTarget => 4,
    }
}

#[cfg(test)]
type FixedObserver = Option<Box<dyn FnMut(&FixedResource)>>;

#[cfg(test)]
thread_local! {
    static FIXED_OBSERVER: std::cell::RefCell<FixedObserver> =
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

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use crate::filesystem::{FilesystemAuthority, FilesystemError};
    use adocweave::SourceId;
    use adocweave::preprocess::{PreparedAnalysisError, ProjectionFailure};
    use adocweave::{NeverCancel, ParseError};

    use super::{
        ExpandedAnalysisFailure, FIXED_OBSERVER, FixedInspection, FixedResource,
        cached_for_authority, cached_inspection_for_authority, derive_confined_authority,
        map_prepared_error, map_projection_failure,
    };
    #[cfg(target_os = "linux")]
    use super::{FixedReadRequest, RequestFilesystemUsage, read_fixed_from};
    use crate::{
        ProjectAuthority, ProjectConfigOverrides, ProjectConfigSelection, ProjectLimits,
        ProjectParseError, ProjectRequest, ProjectResourceFailure, ProjectResourceLimits,
        ProjectResourceOutcome, ProjectTarget, process,
    };

    #[test]
    fn cancellation_mappings_never_create_partial_target_errors() {
        assert_eq!(ProjectParseError::from_parse(ParseError::Cancelled), None);
        assert!(matches!(
            map_prepared_error(PreparedAnalysisError::Cancelled),
            ExpandedAnalysisFailure::Cancelled
        ));
        assert!(matches!(
            map_prepared_error(PreparedAnalysisError::Parse(ParseError::Cancelled)),
            ExpandedAnalysisFailure::Cancelled
        ));
        assert!(matches!(
            map_projection_failure(ProjectionFailure::Cancelled),
            ExpandedAnalysisFailure::Cancelled
        ));
    }

    fn filesystem_authority(roots: impl IntoIterator<Item = PathBuf>) -> FilesystemAuthority {
        FilesystemAuthority::open(roots).expect("filesystem authority")
    }

    fn source_id() -> SourceId {
        SourceId::new("project:cached")
    }

    fn failed_outcome() -> ProjectResourceOutcome {
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(
            crate::ProjectResourceError::from_filesystem(FilesystemError::Unverifiable(
                "fixed failure".to_owned(),
            )),
        ))
    }

    fn assert_rejected(outcome: &ProjectResourceOutcome) {
        assert!(matches!(
            outcome,
            ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(error))
                if error.code == crate::ProjectResourceErrorCode::OutsideAuthority
        ));
    }

    fn loaded_resource(
        requested_path: PathBuf,
        path: PathBuf,
        filesystem: &FilesystemAuthority,
    ) -> FixedResource {
        let authority = filesystem.authority_for_path(&requested_path).cloned();
        FixedResource {
            source_id: source_id(),
            requested_path,
            base: path.parent().map(Path::to_owned),
            path,
            no_symlinks: false,
            authority,
            outcome: ProjectResourceOutcome::Loaded {
                source: Arc::from("loaded"),
            },
            origin: crate::ProjectResourceOrigin::Filesystem,
            observed_bytes: 0,
        }
    }

    fn present_inspection(
        requested_path: PathBuf,
        path: PathBuf,
        filesystem: &FilesystemAuthority,
    ) -> FixedInspection {
        let authority = filesystem.authority_for_path(&requested_path).cloned();
        FixedInspection {
            source_id: source_id(),
            requested_path,
            path,
            authority,
            outcome: ProjectResourceOutcome::Present,
        }
    }

    #[test]
    fn cached_authority_rejects_both_directions_between_independent_roots() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::create_dir(&first).expect("first root");
        fs::create_dir(&second).expect("second root");
        let filesystem = filesystem_authority([first.clone(), second.clone()]);

        for (acquired_root, other_root) in [(&first, &second), (&second, &first)] {
            let acquired_request = acquired_root.join("resource");
            let other_request = other_root.join("resource");
            let loaded =
                loaded_resource(acquired_request.clone(), other_request.clone(), &filesystem);
            assert_rejected(
                &cached_for_authority(&loaded, &acquired_request, &filesystem, false).outcome,
            );

            let present =
                present_inspection(acquired_request.clone(), other_request.clone(), &filesystem);
            assert_rejected(
                &cached_inspection_for_authority(&present, &acquired_request, &filesystem).outcome,
            );

            for outcome in [ProjectResourceOutcome::Missing, failed_outcome()] {
                let fixed = FixedResource {
                    source_id: source_id(),
                    requested_path: acquired_request.clone(),
                    path: acquired_request.clone(),
                    base: None,
                    no_symlinks: false,
                    authority: filesystem.authority_for_path(&acquired_request).cloned(),
                    outcome,
                    origin: crate::ProjectResourceOrigin::Filesystem,
                    observed_bytes: 0,
                };
                assert_rejected(
                    &cached_for_authority(&fixed, &other_request, &filesystem, false).outcome,
                );
            }
        }
    }

    #[test]
    fn cached_authority_rejects_parent_observations_after_narrowing_to_the_same_path() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let parent = directory.path().to_owned();
        let child = directory.path().join("child");
        fs::create_dir(&child).expect("child root");
        let path = child.join("resource");
        let mut policy = FilesystemAuthority::open([parent.clone()]).expect("parent authority");
        let parent_filesystem = policy.clone();
        let child_filesystem = policy
            .derive(&parent, [child])
            .expect("derived child authority");

        let loaded = loaded_resource(path.clone(), path.clone(), &parent_filesystem);
        assert_rejected(&cached_for_authority(&loaded, &path, &child_filesystem, false).outcome);
        let present = present_inspection(path.clone(), path.clone(), &parent_filesystem);
        assert_rejected(
            &cached_inspection_for_authority(&present, &path, &child_filesystem).outcome,
        );
    }

    #[test]
    fn cached_authority_rejects_child_observations_under_a_parent_only_authority() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let parent = directory.path().to_owned();
        let child = parent.join("child");
        fs::create_dir(&child).expect("child root");
        let path = child.join("resource");
        let policy = FilesystemAuthority::open([parent.clone(), child]).expect("nested authority");
        let nested_filesystem = policy.clone();
        let parent_filesystem = policy.select([parent]).expect("parent access");

        let loaded = loaded_resource(path.clone(), path.clone(), &nested_filesystem);
        assert_rejected(&cached_for_authority(&loaded, &path, &parent_filesystem, false).outcome);
        let present = present_inspection(path.clone(), path.clone(), &nested_filesystem);
        assert_rejected(
            &cached_inspection_for_authority(&present, &path, &parent_filesystem).outcome,
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cached_parent_observation_is_rejected_after_the_child_directory_is_replaced() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let parent = directory.path().to_owned();
        let child = parent.join("child");
        let old_child = parent.join("old-child");
        fs::create_dir(&child).expect("child root");
        let path = child.join("resource");
        fs::write(&path, "old").expect("old resource");
        let mut policy = FilesystemAuthority::open([parent.clone()]).expect("parent authority");
        let parent_filesystem = policy.clone();
        let loaded = loaded_resource(path.clone(), path.clone(), &parent_filesystem);

        fs::rename(&child, &old_child).expect("replace old child root");
        fs::create_dir(&child).expect("replacement child root");
        fs::write(child.join("resource"), "new").expect("replacement resource");
        let child_filesystem = policy
            .derive(&parent, [child])
            .expect("replacement child authority");

        assert_rejected(&cached_for_authority(&loaded, &path, &child_filesystem, false).outcome);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn canonical_body_is_not_reused_by_an_independently_opened_authority() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("project");
        let displaced = directory.path().join("opened-project");
        fs::create_dir(&root).expect("project root");
        let path = root.join("guide.adoc");
        fs::write(&path, "old body").expect("old body");
        let first = filesystem_authority([root.clone()]);
        let limits = ProjectLimits::default();
        let mut fixed = BTreeMap::new();
        let inspections = BTreeMap::new();
        let mut usage = RequestFilesystemUsage::default();
        let old = read_fixed_from(
            &mut fixed,
            &inspections,
            limits,
            FixedReadRequest {
                source_id: SourceId::new("old"),
                path: path.clone(),
                allowance: None,
                no_symlinks: false,
            },
            &first,
            &mut usage,
        );
        assert!(matches!(
            old.outcome,
            ProjectResourceOutcome::Loaded { ref source } if source.as_ref() == "old body"
        ));

        fs::rename(&root, &displaced).expect("displace opened root");
        fs::create_dir(&root).expect("replacement root");
        fs::write(&path, "new body").expect("new body");
        let second = filesystem_authority([root]);
        let new = read_fixed_from(
            &mut fixed,
            &inspections,
            limits,
            FixedReadRequest {
                source_id: SourceId::new("new"),
                path,
                allowance: None,
                no_symlinks: false,
            },
            &second,
            &mut usage,
        );
        assert!(matches!(
            new.outcome,
            ProjectResourceOutcome::Loaded { ref source } if source.as_ref() == "new body"
        ));
        assert_eq!(usage.read_operations, 2);
    }

    #[test]
    fn confined_authority_derivation_is_transactional() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let parent = directory.path().join("parent");
        let child = parent.join("child");
        let outside = directory.path().join("outside");
        fs::create_dir_all(&child).expect("child root");
        fs::create_dir(&outside).expect("outside root");
        let mut authority = filesystem_authority([parent.clone()]);
        let original = authority.roots().to_vec();

        assert!(matches!(
            derive_confined_authority(&mut authority, &[child.clone(), outside]),
            Err(FilesystemError::OutsideRoot(_))
        ));
        assert_eq!(authority.roots(), original);
        assert!(authority.authority_for_root(&child).is_none());
    }

    #[test]
    fn confined_authority_root_limit_failure_keeps_the_previous_handles() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let parent = directory.path().join("parent");
        fs::create_dir(&parent).expect("parent root");
        let mut children = Vec::new();
        for index in 0..128 {
            let child = parent.join(index.to_string());
            fs::create_dir(&child).expect("child root");
            children.push(child);
        }
        let mut authority = filesystem_authority([parent]);
        derive_confined_authority(&mut authority, &children[..127]).expect("128 retained roots");
        let original = authority.roots().to_vec();

        assert!(matches!(
            derive_confined_authority(&mut authority, &[children[127].clone()]),
            Err(FilesystemError::LimitExceeded { limit: 128 })
        ));
        assert_eq!(authority.roots(), original);
        assert!(authority.authority_for_root(&children[127]).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn processing_reinspects_a_replaced_child_instead_of_reusing_a_parent_body() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().to_owned();
        let child = root.join("child");
        let old_child = root.join("old-child");
        let resource = child.join("a-resource.adoc");
        fs::create_dir(&child).expect("child directory");
        fs::write(
            root.join(".adocweave.toml"),
            "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \"child\"\n",
        )
        .expect("project configuration");
        fs::write(&resource, "old resource\n").expect("old resource");
        fs::write(child.join("z-guide.adoc"), "old guide\n").expect("old guide");
        let observed_resource = resource.clone();
        let replacement_child = child.clone();
        FIXED_OBSERVER.with(|observer| {
            let mut replaced = false;
            *observer.borrow_mut() = Some(Box::new(move |fixed| {
                if !replaced && fixed.path == observed_resource {
                    fs::rename(&replacement_child, &old_child).expect("move observed child");
                    fs::create_dir(&replacement_child).expect("replacement child");
                    fs::write(
                        replacement_child.join("z-guide.adoc"),
                        "image::a-resource.adoc[]\n",
                    )
                    .expect("replacement guide");
                    replaced = true;
                }
            }));
        });
        let filesystem_reads = ProjectResourceLimits::default();
        let outcome = process(
            ProjectRequest {
                targets: vec![
                    ProjectTarget::Path(PathBuf::from("child/a-resource.adoc")),
                    ProjectTarget::Path(PathBuf::from("child/z-guide.adoc")),
                ],
                sources: Vec::new(),
                config: ProjectConfigSelection::Discover,
                overrides: ProjectConfigOverrides::default(),
                apply_safe_fixes: false,
                resource_selection: crate::ProjectResourceSelection {
                    local_targets: true,
                    stylesheets: false,
                },
                authority: ProjectAuthority::open(root.clone(), [root]).expect("parent authority"),
                limits: ProjectLimits {
                    resources: ProjectResourceLimits {
                        max_files: filesystem_reads.max_files,
                        max_total_bytes: filesystem_reads.max_total_bytes,
                        max_resource_bytes: filesystem_reads.max_resource_bytes,
                    },
                    max_directory_entries: 100,
                    max_processing_iterations: 10,
                    max_output_bytes: u32::MAX,
                },
            },
            &NeverCancel,
        );
        FIXED_OBSERVER.with(|observer| {
            observer.borrow_mut().take();
        });

        let result = outcome.expect("replacement remains inside the request authority");
        let guide = result
            .targets
            .iter()
            .find(|target| {
                target
                    .path
                    .as_ref()
                    .is_some_and(|path| path.ends_with("z-guide.adoc"))
            })
            .expect("replacement guide result");
        assert!(guide.resources.iter().any(|resource| {
            resource.kind == crate::ProjectResourceKind::LocalTarget
                && resource.path.ends_with("a-resource.adoc")
                && resource.outcome == ProjectResourceOutcome::Missing
        }));
    }

    #[test]
    fn same_root_reuses_body_and_inspection_outcomes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().to_owned();
        let path = root.join("resource");
        let filesystem = filesystem_authority([root.clone()]);

        for outcome in [
            ProjectResourceOutcome::Loaded {
                source: Arc::from("loaded"),
            },
            ProjectResourceOutcome::Present,
            ProjectResourceOutcome::Missing,
            failed_outcome(),
        ] {
            let fixed = FixedResource {
                source_id: source_id(),
                requested_path: path.clone(),
                path: path.clone(),
                base: path.parent().map(Path::to_owned),
                no_symlinks: false,
                authority: filesystem.authority_for_path(&path).cloned(),
                outcome: outcome.clone(),
                origin: crate::ProjectResourceOrigin::Filesystem,
                observed_bytes: 0,
            };
            assert_eq!(
                cached_for_authority(&fixed, &path, &filesystem, false).outcome,
                outcome
            );
        }

        for outcome in [
            ProjectResourceOutcome::Present,
            ProjectResourceOutcome::Missing,
            failed_outcome(),
        ] {
            let fixed = FixedInspection {
                source_id: source_id(),
                requested_path: path.clone(),
                path: path.clone(),
                authority: filesystem.authority_for_path(&path).cloned(),
                outcome: outcome.clone(),
            };
            assert_eq!(
                cached_inspection_for_authority(&fixed, &path, &filesystem).outcome,
                outcome
            );
        }
    }

    #[test]
    fn first_observation_is_fixed_when_an_include_changes_during_processing() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().to_owned();
        let include = root.join("part.adoc");
        fs::write(
            root.join("guide.adoc"),
            "include::part.adoc[]\n\ninclude::part.adoc[]\n",
        )
        .expect("primary fixture");
        fs::write(&include, "old content\n").expect("include fixture");
        let include_for_observer = include.clone();
        FIXED_OBSERVER.with(|observer| {
            *observer.borrow_mut() = Some(Box::new(move |fixed| {
                if fixed.path == include_for_observer {
                    fs::write(&include_for_observer, "new content\n")
                        .expect("include changes after observation");
                }
            }));
        });
        let filesystem_reads = ProjectResourceLimits::default();
        let result = process(
            ProjectRequest {
                targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
                sources: Vec::new(),
                config: ProjectConfigSelection::Disabled,
                overrides: ProjectConfigOverrides {
                    include: Some(true),
                    enable_lint_rules: Vec::new(),
                    resource_roots: None,
                    local_target_project_root: None,
                    stylesheet_files: Vec::new(),
                },
                apply_safe_fixes: false,
                resource_selection: Default::default(),
                authority: ProjectAuthority::open(root.clone(), [root])
                    .expect("temporary project authority"),
                limits: ProjectLimits {
                    resources: ProjectResourceLimits {
                        max_files: filesystem_reads.max_files,
                        max_total_bytes: filesystem_reads.max_total_bytes,
                        max_resource_bytes: filesystem_reads.max_resource_bytes,
                    },
                    max_directory_entries: 100,
                    max_processing_iterations: 10,
                    max_output_bytes: u32::MAX,
                },
            },
            &NeverCancel,
        )
        .expect("processing succeeds");
        FIXED_OBSERVER.with(|observer| *observer.borrow_mut() = None);

        let source = &result.targets[0]
            .analysis
            .as_ref()
            .expect("analysis succeeds")
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .preprocessed
            .document
            .source;
        assert_eq!(source.matches("old content").count(), 2);
        assert!(!source.contains("new content"));
    }
}
