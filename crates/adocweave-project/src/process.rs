use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adocweave::preprocess::{
    EffectivePreprocessStep, EffectiveProcessingOptions, PreparedAnalysisError, PreprocessInputs,
    PreprocessedAnalysis, PreprocessedAnalysisError, ProjectionLimits, ResourceDocument,
    ResourceLookup, ResourceLookupResult,
};
use adocweave::{NeverCancel, SourceId};
use adocweave_config::{ConfigSnapshot, ResolvedProjectConfig};
use adocweave_host::{
    DerivedFilesystemRoots, FilesystemJobError, FilesystemJobLimit, FilesystemJobLimits,
    IncludeFilesystemBudgetedOutcome, IncludeFilesystemInspectionOutcome, IncludeFilesystemJob,
    IncludeFilesystemPathRequest, IncludeFilesystemRequest, LocalFilesystemPolicy,
    LocalFilesystemSession, LogicalSourceId, ResourceError,
};

use crate::selection::{absolute_lexical, identity_path, scan_root_for_selector, select_targets};
use crate::{
    ConfigSelection, ProjectError, ProjectLimit, ProjectOutcome, ProjectRequest,
    ProjectResourceFailure, ProjectResourceKind, ProjectResourceOutcome, ProjectResourceResult,
    ProjectResult, ProjectTargetError, ProjectTargetResult, ProjectUsage, ProjectWarning,
};

pub fn process(request: ProjectRequest) -> ProjectOutcome {
    Processor::new(request)?.run()
}

struct Processor {
    project_root: PathBuf,
    config_selection: ConfigSelection,
    overrides: crate::ProjectOverrides,
    limits: crate::ProjectLimits,
    selectors: Vec<crate::ProjectTarget>,
    authority: LocalFilesystemPolicy,
    identity_roots: Vec<PathBuf>,
    filesystem: LocalFilesystemSession,
    job: IncludeFilesystemJob,
    fixed: BTreeMap<LogicalSourceId, FixedResource>,
    inspections: BTreeMap<LogicalSourceId, FixedInspection>,
    configs: BTreeMap<PathBuf, ConfigSnapshot>,
    processing_iterations: u32,
    output_bytes: u64,
    warnings: Vec<ProjectWarning>,
}

#[derive(Clone, Debug)]
struct FixedResource {
    source_id: LogicalSourceId,
    requested_path: PathBuf,
    path: PathBuf,
    base: Option<PathBuf>,
    no_symlinks: bool,
    outcome: ProjectResourceOutcome,
}

#[derive(Clone, Debug)]
struct FixedInspection {
    source_id: LogicalSourceId,
    path: PathBuf,
    outcome: ProjectResourceOutcome,
}

struct TargetAnalysisContext<'target> {
    source_id: &'target LogicalSourceId,
    source: &'target Arc<str>,
    config: &'target ResolvedProjectConfig,
    allowed_roots: &'target [PathBuf],
    budget: &'target mut TargetBudget,
    bases: &'target mut BTreeMap<String, PathBuf>,
    lookup_bases: &'target mut BTreeMap<String, PathBuf>,
    resources: &'target mut Vec<ProjectResourceResult>,
    filesystem: &'target mut LocalFilesystemSession,
}

impl FixedInspection {
    fn result(&self, requested_by: Option<LogicalSourceId>) -> ProjectResourceResult {
        ProjectResourceResult {
            source_id: self.source_id.clone(),
            path: self.path.clone(),
            kind: ProjectResourceKind::LocalTarget,
            requested_by,
            outcome: self.outcome.clone(),
        }
    }
}

impl FixedResource {
    fn result(
        &self,
        kind: ProjectResourceKind,
        requested_by: Option<LogicalSourceId>,
    ) -> ProjectResourceResult {
        ProjectResourceResult {
            source_id: self.source_id.clone(),
            path: self.path.clone(),
            kind,
            requested_by,
            outcome: self.outcome.clone(),
        }
    }
}

impl Processor {
    fn new(request: ProjectRequest) -> Result<Self, ProjectError> {
        let ProjectRequest {
            project_root,
            targets,
            config,
            overrides,
            mut authority,
            limits,
        } = request;
        let project_root = absolute_lexical(
            authority
                .roots()
                .first()
                .map(PathBuf::as_path)
                .unwrap_or_else(|| Path::new("")),
            &project_root,
        )
        .map_err(ProjectError::Authority)?;
        let root_policy = authority
            .policy_for_path(&project_root)
            .cloned()
            .ok_or_else(|| {
                ProjectError::Authority(ResourceError::OutsideRoots(project_root.clone()))
            })?;
        let project_policy = if root_policy.root() == project_root {
            root_policy
        } else {
            root_policy
                .derive_confined_directory(&project_root)
                .map_err(ResourceError::from)
                .map_err(ProjectError::Authority)?
        };
        let project_root = project_policy.root().to_owned();
        let retained_roots = authority.roots().to_vec();
        let identity_roots = retained_roots.clone();
        authority = authority
            .access_existing(retained_roots, limits.filesystem_reads)
            .map_err(ProjectError::Authority)?;
        let filesystem = authority.session().map_err(ProjectError::Authority)?;
        let read_operations = u64::try_from(limits.filesystem_reads.max_files).unwrap_or(u64::MAX);
        // One scan session per selector, one common read session and at most one
        // confined local-target session per document admitted by the file limit.
        let max_sessions = limits
            .filesystem_reads
            .max_files
            .saturating_mul(2)
            .saturating_add(targets.len())
            .saturating_add(1);
        let job = IncludeFilesystemJob::new(FilesystemJobLimits {
            max_read_operations: read_operations,
            max_read_bytes: limits.filesystem_reads.max_total_bytes,
            max_read_probe_bytes: read_operations.max(1),
            max_directory_operations: limits
                .max_directory_entries
                .saturating_add(u64::try_from(max_sessions).unwrap_or(u64::MAX)),
            max_directory_entries: limits.max_directory_entries,
            max_directory_probe_entries: u64::try_from(max_sessions).unwrap_or(u64::MAX).max(1),
            max_candidate_changes: read_operations,
            max_sessions,
        })
        .map_err(|error| ProjectError::Authority(ResourceError::Job(error)))?;
        Ok(Self {
            project_root,
            config_selection: config,
            overrides,
            limits,
            selectors: targets,
            authority,
            identity_roots,
            filesystem,
            job,
            fixed: BTreeMap::new(),
            inspections: BTreeMap::new(),
            configs: BTreeMap::new(),
            processing_iterations: 0,
            output_bytes: 0,
            warnings: Vec::new(),
        })
    }

    fn run(mut self) -> ProjectOutcome {
        let selectors = self.selectors.clone();
        let mut scan_settings = BTreeMap::new();
        for selector in &selectors {
            if let Some(root) = scan_root_for_selector(&self.project_root, selector)? {
                let (_, config, _) = self.resolve_config_at(&root, true)?;
                scan_settings.insert(root, config.workspace.scan);
            }
        }
        let paths = select_targets(
            &self.project_root,
            &selectors,
            &mut self.authority,
            self.limits,
            &self.job,
            &scan_settings,
            &mut self.warnings,
        )?;
        let mut targets = Vec::with_capacity(paths.len());
        for path in paths {
            targets.push(self.process_target(path)?);
        }
        let processing_iterations = self.processing_iterations;
        let output_bytes = self.output_bytes;
        let mut warnings = self.warnings;
        warnings.sort_by_key(|warning| format!("{warning:?}"));
        drop(self.filesystem);
        let filesystem = self.job.finish().map_err(|error| match error {
            FilesystemJobError::Limit(limit) => {
                ProjectError::Limit(map_job_limit(limit, self.limits))
            }
            error => ProjectError::Authority(ResourceError::Job(error)),
        })?;
        Ok(ProjectResult {
            targets,
            warnings,
            usage: ProjectUsage {
                filesystem,
                processing_iterations,
                output_bytes,
            },
        })
    }

    fn process_target(&mut self, path: PathBuf) -> Result<ProjectTargetResult, ProjectError> {
        let (config_snapshot, mut config, config_resource) =
            self.resolve_config_at(&path, false)?;
        if let Some(include) = self.overrides.include {
            config.resources.include = include;
            config.preprocess.enable_includes = include;
        }
        let source_id = self.source_id_for_path(&path)?;
        let primary = self.read_fixed(source_id.clone(), path.clone());
        let mut resources = config_resource.into_iter().collect::<Vec<_>>();
        resources.push(primary.result(ProjectResourceKind::Primary, None));
        let source = match &primary.outcome {
            ProjectResourceOutcome::Loaded { source } => Arc::clone(source),
            ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                return Ok(self.finish_target(
                    source_id,
                    path,
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Incomplete(*limit)),
                ));
            }
            ProjectResourceOutcome::Missing => {
                return Ok(self.finish_target(
                    source_id,
                    path.clone(),
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Read(ResourceError::Missing(path))),
                ));
            }
            ProjectResourceOutcome::Failed(failure) => {
                return Ok(self.finish_target(
                    source_id,
                    path,
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Read(failure.error().clone())),
                ));
            }
            ProjectResourceOutcome::Present => unreachable!("a primary document is read"),
            ProjectResourceOutcome::LoadedOmitted { .. } => {
                unreachable!("fixed primary resources retain acquired content")
            }
        };
        let mut target_budget = TargetBudget::new(config.resources.limit_plan.filesystem_reads);
        if let Err(limit) = target_budget.charge(&source_id, source.len()) {
            return Ok(self.finish_target(
                source_id,
                path,
                config_snapshot,
                config,
                resources,
                Err(ProjectTargetError::Incomplete(limit)),
            ));
        }
        let mut bases = BTreeMap::from([(
            source_id.as_str().to_owned(),
            primary
                .base
                .clone()
                .unwrap_or_else(|| self.project_root.clone()),
        )]);
        let mut lookup_bases = BTreeMap::from([(
            "__adocweave_base__".to_owned(),
            primary
                .base
                .clone()
                .unwrap_or_else(|| self.project_root.clone()),
        )]);
        let base = primary.base.as_deref().unwrap_or(&path);
        let allowed_roots = if config.resources.include {
            include_roots(base, &config)
        } else {
            vec![base.to_owned()]
        };
        let mut include_filesystem = match self.confined_session(&allowed_roots) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                return Ok(self.finish_target(
                    source_id,
                    path,
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Read(error)),
                ));
            }
        };
        let outcome = self.analyze_target(TargetAnalysisContext {
            source_id: &source_id,
            source: &source,
            config: &config,
            allowed_roots: &allowed_roots,
            budget: &mut target_budget,
            bases: &mut bases,
            lookup_bases: &mut lookup_bases,
            resources: &mut resources,
            filesystem: &mut include_filesystem,
        });
        if let Ok(analysis) = &outcome {
            self.collect_local_targets(analysis, &config, &bases, &mut resources);
        }
        self.collect_stylesheets(&source_id, &config, &mut resources);
        Ok(self.finish_target(source_id, path, config_snapshot, config, resources, outcome))
    }

    fn finish_target(
        &mut self,
        source_id: LogicalSourceId,
        path: PathBuf,
        config: Option<ConfigSnapshot>,
        resolved_config: ResolvedProjectConfig,
        mut resources: Vec<ProjectResourceResult>,
        mut outcome: Result<PreprocessedAnalysis, ProjectTargetError>,
    ) -> ProjectTargetResult {
        resources.sort_by(|left, right| {
            left.source_id
                .cmp(&right.source_id)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| resource_kind_order(left.kind).cmp(&resource_kind_order(right.kind)))
                .then_with(|| left.requested_by.cmp(&right.requested_by))
        });
        let returned_bytes = outcome
            .as_ref()
            .map_or(0, |analysis| analysis.document.source.len())
            .saturating_add(
                resources
                    .iter()
                    .map(|resource| match &resource.outcome {
                        ProjectResourceOutcome::Loaded { source } => source.len(),
                        ProjectResourceOutcome::LoadedOmitted { .. } => 0,
                        _ => 0,
                    })
                    .sum::<usize>(),
            );
        let total = self
            .output_bytes
            .saturating_add(u64::try_from(returned_bytes).unwrap_or(u64::MAX));
        if total > u64::from(self.limits.output.max_output_bytes) {
            let limit = ProjectLimit::OutputBytes {
                limit: self.limits.output.max_output_bytes,
            };
            for resource in &mut resources {
                if matches!(resource.outcome, ProjectResourceOutcome::Loaded { .. }) {
                    resource.outcome = ProjectResourceOutcome::LoadedOmitted { limit };
                }
            }
            outcome = Err(ProjectTargetError::Incomplete(limit));
        } else {
            self.output_bytes = total;
        }
        target_result(source_id, path, config, resolved_config, resources, outcome)
    }

    fn resolve_config_at(
        &mut self,
        target: &Path,
        target_is_directory: bool,
    ) -> Result<
        (
            Option<ConfigSnapshot>,
            ResolvedProjectConfig,
            Option<ProjectResourceResult>,
        ),
        ProjectError,
    > {
        let path = match self.config_selection.clone() {
            ConfigSelection::Disabled => None,
            ConfigSelection::Explicit(path) => {
                Some(absolute_lexical(&self.project_root, &path).map_err(ProjectError::Authority)?)
            }
            ConfigSelection::Discover => self.discover_config(target, target_is_directory)?,
        };
        let Some(path) = path else {
            return Ok((None, ResolvedProjectConfig::default(), None));
        };
        let snapshot = self.load_config(path.clone())?;
        if !snapshot.path.starts_with(&self.project_root) {
            for configured in snapshot
                .config
                .resources
                .roots
                .iter()
                .chain(&snapshot.config.html.stylesheet_files)
                .chain(snapshot.config.local_targets.project_root.iter())
            {
                if !configured.starts_with(&self.project_root) {
                    return Err(ProjectError::Authority(ResourceError::OutsideRoots(
                        configured.clone(),
                    )));
                }
            }
        }
        let source_id = self.source_id_for_path(&path)?;
        let resource = self
            .fixed
            .get(&source_id)
            .map(|fixed| fixed.result(ProjectResourceKind::Config, None));
        Ok((Some(snapshot.clone()), snapshot.config.clone(), resource))
    }

    fn load_config(&mut self, path: PathBuf) -> Result<ConfigSnapshot, ProjectError> {
        if let Some(snapshot) = self.configs.get(&path) {
            return Ok(snapshot.clone());
        }
        let source_id = self.source_id_for_path(&path)?;
        let fixed = self.read_fixed_no_symlinks(source_id, path.clone());
        let ProjectResourceOutcome::Loaded { source } = &fixed.outcome else {
            return match &fixed.outcome {
                ProjectResourceOutcome::Missing => {
                    Err(ProjectError::Authority(ResourceError::Missing(path)))
                }
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                    Err(ProjectError::Limit(*limit))
                }
                ProjectResourceOutcome::Failed(failure) => {
                    Err(ProjectError::Authority(failure.error().clone()))
                }
                ProjectResourceOutcome::Present
                | ProjectResourceOutcome::Loaded { .. }
                | ProjectResourceOutcome::LoadedOmitted { .. } => {
                    unreachable!("configuration reads return content, absence or failure")
                }
            };
        };
        let snapshot =
            adocweave_config::ConfigSnapshot::from_utf8_source(fixed.path.clone(), source)
                .map_err(ProjectError::Config)?;
        self.configs.insert(path, snapshot.clone());
        Ok(snapshot)
    }

    fn discover_config(
        &mut self,
        target: &Path,
        target_is_directory: bool,
    ) -> Result<Option<PathBuf>, ProjectError> {
        let mut directory = if !target.starts_with(&self.project_root) {
            self.project_root.clone()
        } else if target_is_directory {
            target.to_owned()
        } else {
            target
                .parent()
                .map(Path::to_owned)
                .unwrap_or_else(|| self.project_root.clone())
        };
        loop {
            let candidate = directory.join(adocweave_config::FILE_NAME);
            let source_id = self.source_id_for_path(&candidate)?;
            let fixed = self.read_fixed_no_symlinks(source_id, candidate.clone());
            match fixed.outcome {
                ProjectResourceOutcome::Loaded { .. } => return Ok(Some(fixed.path)),
                ProjectResourceOutcome::Missing => {}
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                    return Err(ProjectError::Limit(limit));
                }
                ProjectResourceOutcome::Failed(failure) => {
                    return Err(ProjectError::Authority(failure.error().clone()));
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

    fn read_fixed_no_symlinks(
        &mut self,
        source_id: LogicalSourceId,
        path: PathBuf,
    ) -> FixedResource {
        if let Some(fixed) = self.fixed.get(&source_id) {
            return cached_for_session(fixed, &self.filesystem, true);
        }
        let transaction = self
            .job
            .transaction(&self.filesystem)
            .map_err(ResourceError::from);
        let outcome = transaction.and_then(|mut transaction| {
            match transaction.read_utf8_no_symlinks_within_budget(
                IncludeFilesystemPathRequest::new(source_id.clone(), path.clone()),
            ) {
                IncludeFilesystemBudgetedOutcome::Found(found) => {
                    let canonical = found.provenance().canonical_path().to_owned();
                    let source = Arc::<str>::from(found.source());
                    transaction
                        .commit(&mut self.filesystem)
                        .map_err(ResourceError::from)?;
                    Ok((
                        canonical.clone(),
                        canonical.parent().map(Path::to_owned),
                        ProjectResourceOutcome::Loaded { source },
                    ))
                }
                IncludeFilesystemBudgetedOutcome::NotFound(missing) => {
                    transaction
                        .commit(&mut self.filesystem)
                        .map_err(ResourceError::from)?;
                    Ok((
                        missing.watch_candidate().path().to_owned(),
                        None,
                        ProjectResourceOutcome::Missing,
                    ))
                }
                IncludeFilesystemBudgetedOutcome::BudgetExhausted { .. } => Ok((
                    path.clone(),
                    None,
                    ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                        self.current_read_limit(),
                    )),
                )),
                IncludeFilesystemBudgetedOutcome::Failed(failed) => {
                    Err(ResourceError::from(failed.error().clone()))
                }
            }
        });
        let (path, base, outcome) = outcome.unwrap_or_else(|error| {
            (
                path,
                None,
                ProjectResourceOutcome::Failed(classify_resource_failure(error, self.limits)),
            )
        });
        let fixed = FixedResource {
            source_id: source_id.clone(),
            requested_path: path.clone(),
            path,
            base,
            no_symlinks: true,
            outcome,
        };
        self.fixed.insert(source_id, fixed.clone());
        observe_fixed(&fixed);
        fixed
    }

    fn analyze_target(
        &mut self,
        context: TargetAnalysisContext<'_>,
    ) -> Result<PreprocessedAnalysis, ProjectTargetError> {
        let TargetAnalysisContext {
            source_id,
            source,
            config,
            allowed_roots,
            budget,
            bases,
            lookup_bases,
            resources,
            filesystem,
        } = context;
        let mut preprocess = config.preprocess.clone();
        preprocess.enable_includes = config.resources.include;
        preprocess.source_id = Some(SourceId::new(source_id.as_str()));
        preprocess.base_uri = Some("__adocweave_base__".to_owned());
        let options = EffectiveProcessingOptions::new(config.analysis.clone(), preprocess)
            .map_err(|error| {
                ProjectTargetError::Analysis(PreprocessedAnalysisError::Options(error))
            })?;
        let mut lookup = FixedLookup::default();
        self.next_iteration()
            .map_err(ProjectTargetError::Incomplete)?;
        let mut step = options.preprocess_resumable(source, &lookup, &NeverCancel);
        loop {
            match step {
                EffectivePreprocessStep::Complete(prepared) => {
                    return options
                        .analyze_preprocessed(prepared, PreprocessInputs::default())
                        .map_err(map_prepared_error);
                }
                EffectivePreprocessStep::NeedResource(suspended) => {
                    self.next_iteration()
                        .map_err(ProjectTargetError::Incomplete)?;
                    let request = suspended.request();
                    let target = request.target().to_owned();
                    let requested_by = request
                        .source_id()
                        .map(|id| self.source_id_for_value(id.as_str()))
                        .transpose()
                        .map_err(ProjectTargetError::Read)?;
                    let path = resolve_lookup_path(&target, lookup_bases)
                        .map_err(ProjectTargetError::Read)?;
                    let include_id = self
                        .source_id_for_path(&path)
                        .map_err(|error| ProjectTargetError::Read(project_error_resource(error)))?;
                    if !allowed_roots.iter().any(|root| path.starts_with(root)) {
                        let error = ResourceError::OutsideRoots(path.clone());
                        let fixed = self.fix_failure(
                            include_id,
                            path,
                            ProjectResourceFailure::Rejected(error.clone()),
                        );
                        resources.push(fixed.result(ProjectResourceKind::Include, requested_by));
                        return Err(ProjectTargetError::Read(error));
                    }
                    let fixed = read_fixed_from(
                        &self.job,
                        &mut self.fixed,
                        self.limits,
                        include_id.clone(),
                        path,
                        filesystem,
                    );
                    resources
                        .push(fixed.result(ProjectResourceKind::Include, requested_by.clone()));
                    let response = match &fixed.outcome {
                        ProjectResourceOutcome::Loaded { source } => {
                            budget
                                .charge(&include_id, source.len())
                                .map_err(ProjectTargetError::Incomplete)?;
                            if let Some(base) = &fixed.base {
                                bases.insert(include_id.as_str().to_owned(), base.clone());
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
                            lookup
                                .entries
                                .insert(target.clone(), ResourceLookupResult::Missing);
                            request.not_found()
                        }
                        ProjectResourceOutcome::Failed(failure) => {
                            let message = failure.to_string();
                            lookup.entries.insert(
                                target.clone(),
                                ResourceLookupResult::Failed(message.clone()),
                            );
                            request.load_failed(message)
                        }
                        ProjectResourceOutcome::Present => unreachable!("an include is read"),
                        ProjectResourceOutcome::LoadedOmitted { .. } => {
                            unreachable!("fixed include resources retain acquired content")
                        }
                    };
                    step = suspended.resume(response, &lookup, &NeverCancel);
                }
                EffectivePreprocessStep::Failed(error) => {
                    return Err(ProjectTargetError::Analysis(
                        PreprocessedAnalysisError::Preprocess(error),
                    ));
                }
                EffectivePreprocessStep::HostError(error) => {
                    return Err(ProjectTargetError::Read(ResourceError::Unverifiable(
                        error.to_string(),
                    )));
                }
                EffectivePreprocessStep::Cancelled => {
                    return Err(ProjectTargetError::Analysis(
                        PreprocessedAnalysisError::Cancelled,
                    ));
                }
                _ => {
                    return Err(ProjectTargetError::Read(ResourceError::Unverifiable(
                        "unknown preprocessing state".to_owned(),
                    )));
                }
            }
        }
    }

    fn collect_stylesheets(
        &mut self,
        requested_by: &LogicalSourceId,
        config: &ResolvedProjectConfig,
        resources: &mut Vec<ProjectResourceResult>,
    ) {
        for path in &config.html.stylesheet_files {
            let source_id = match self.source_id_for_path(path) {
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
                    self.warnings.push(ProjectWarning::LocalTargetProjection {
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let fixed = self.read_fixed(source_id, path.clone());
            resources
                .push(fixed.result(ProjectResourceKind::Stylesheet, Some(requested_by.clone())));
        }
    }

    fn collect_local_targets(
        &mut self,
        analysis: &PreprocessedAnalysis,
        config: &ResolvedProjectConfig,
        bases: &BTreeMap<String, PathBuf>,
        resources: &mut Vec<ProjectResourceResult>,
    ) {
        if !config.local_targets.enabled {
            return;
        }
        let Some(authority) = config.local_targets.project_root.as_deref() else {
            return;
        };
        let mut local_filesystem = match self.confined_session(&[authority.to_owned()]) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                self.warnings.push(ProjectWarning::Resource {
                    path: authority.to_owned(),
                    kind: ProjectResourceKind::LocalTarget,
                    failure: ProjectResourceFailure::Rejected(error),
                });
                return;
            }
        };
        let projection = match analysis.project_origins(ProjectionLimits::default()) {
            Ok(projection) => projection,
            Err(error) => {
                self.warnings.push(ProjectWarning::LocalTargetProjection {
                    message: error.to_string(),
                });
                return;
            }
        };
        let mut candidates = Vec::new();
        for target in projection.local_targets {
            let owner = target
                .origins
                .first()
                .and_then(|origin| origin.source_id.as_ref())
                .map_or_else(
                    || analysis.analysis.source_id().map(SourceId::as_str),
                    |source| Some(source.as_str()),
                );
            let Some(owner) = owner else {
                self.warnings.push(ProjectWarning::LocalTargetProjection {
                    message: "local target has no source owner".to_owned(),
                });
                continue;
            };
            let Some(base) = bases.get(owner).cloned() else {
                self.warnings.push(ProjectWarning::LocalTargetProjection {
                    message: format!("local target owner has no verified base: {owner}"),
                });
                continue;
            };
            candidates.push((owner.to_owned(), base, target.value));
        }
        let mut seen = BTreeSet::new();
        for (owner, base, target) in candidates {
            if !seen.insert((owner.clone(), target.path.clone())) {
                continue;
            }
            let requested_by = match self.source_id_for_value(&owner) {
                Ok(source_id) => Some(source_id),
                Err(error) => {
                    self.warnings.push(ProjectWarning::Resource {
                        path: base.clone(),
                        kind: ProjectResourceKind::LocalTarget,
                        failure: ProjectResourceFailure::Rejected(error),
                    });
                    None
                }
            };
            let path = match absolute_lexical(&base, Path::new(&target.path)) {
                Ok(path) => path,
                Err(error) => {
                    match self.source_id_for_value(&target.path) {
                        Ok(source_id) => {
                            let fixed = self.fix_failure(
                                source_id,
                                base.join(&target.path),
                                ProjectResourceFailure::Rejected(error),
                            );
                            resources
                                .push(fixed.result(ProjectResourceKind::LocalTarget, requested_by));
                        }
                        Err(id_error) => self.warnings.push(ProjectWarning::Resource {
                            path: base.join(&target.path),
                            kind: ProjectResourceKind::LocalTarget,
                            failure: ProjectResourceFailure::Rejected(id_error),
                        }),
                    }
                    continue;
                }
            };
            let source_id = match self.source_id_for_path(&path) {
                Ok(source_id) => source_id,
                Err(ProjectError::Authority(error)) => {
                    self.warnings.push(ProjectWarning::Resource {
                        path,
                        kind: ProjectResourceKind::LocalTarget,
                        failure: ProjectResourceFailure::Rejected(error),
                    });
                    continue;
                }
                Err(error) => {
                    self.warnings.push(ProjectWarning::LocalTargetProjection {
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let result = if target.syntax == adocweave::LocalTargetSyntax::Unverifiable {
                self.fix_failure(
                    source_id,
                    path,
                    ProjectResourceFailure::Rejected(ResourceError::Unverifiable(target.target)),
                )
                .result(ProjectResourceKind::LocalTarget, requested_by)
            } else {
                self.inspect_fixed_in(
                    source_id,
                    path,
                    authority,
                    &base,
                    &target.path,
                    &mut local_filesystem,
                )
                .result(requested_by)
            };
            resources.push(result);
        }
    }

    fn read_fixed(&mut self, source_id: LogicalSourceId, path: PathBuf) -> FixedResource {
        read_fixed_from(
            &self.job,
            &mut self.fixed,
            self.limits,
            source_id,
            path,
            &mut self.filesystem,
        )
    }

    fn inspect_fixed_in(
        &mut self,
        source_id: LogicalSourceId,
        path: PathBuf,
        authority: &Path,
        base: &Path,
        target: &str,
        filesystem: &mut LocalFilesystemSession,
    ) -> FixedInspection {
        if let Some(fixed) = self.fixed.get(&source_id) {
            let fixed = cached_for_session(fixed, filesystem, false);
            return FixedInspection {
                source_id,
                path: fixed.path.clone(),
                outcome: match &fixed.outcome {
                    ProjectResourceOutcome::Loaded { .. }
                    | ProjectResourceOutcome::LoadedOmitted { .. }
                    | ProjectResourceOutcome::Present => ProjectResourceOutcome::Present,
                    outcome => outcome.clone(),
                },
            };
        }
        if let Some(fixed) = self.inspections.get(&source_id) {
            return fixed.clone();
        }
        let outcome = self
            .job
            .transaction(filesystem)
            .map_err(ResourceError::from)
            .and_then(|mut transaction| {
                match transaction.inspect_within(
                    authority,
                    IncludeFilesystemRequest::new(source_id.clone(), base, target),
                ) {
                    IncludeFilesystemInspectionOutcome::Found(found) => {
                        let canonical = found.provenance().canonical_path().to_owned();
                        transaction
                            .commit(filesystem)
                            .map_err(ResourceError::from)?;
                        Ok((canonical, ProjectResourceOutcome::Present))
                    }
                    IncludeFilesystemInspectionOutcome::NotFound(missing) => {
                        let candidate = missing.watch_candidate().path().to_owned();
                        transaction
                            .commit(filesystem)
                            .map_err(ResourceError::from)?;
                        Ok((candidate, ProjectResourceOutcome::Missing))
                    }
                    IncludeFilesystemInspectionOutcome::Failed(failed) => {
                        Err(ResourceError::from(failed.error().clone()))
                    }
                }
            });
        let (path, outcome) = match outcome {
            Ok(value) => value,
            Err(error) => (
                path,
                ProjectResourceOutcome::Failed(classify_resource_failure(error, self.limits)),
            ),
        };
        let fixed = FixedInspection {
            source_id: source_id.clone(),
            path: path.clone(),
            outcome: outcome.clone(),
        };
        if !matches!(outcome, ProjectResourceOutcome::Present) {
            self.fixed.insert(
                source_id.clone(),
                FixedResource {
                    source_id: source_id.clone(),
                    requested_path: path.clone(),
                    path,
                    base: None,
                    no_symlinks: false,
                    outcome,
                },
            );
        }
        self.inspections.insert(source_id, fixed.clone());
        fixed
    }

    fn fix_failure(
        &mut self,
        source_id: LogicalSourceId,
        path: PathBuf,
        failure: ProjectResourceFailure,
    ) -> FixedResource {
        if let Some(fixed) = self.fixed.get(&source_id) {
            let mut rejected = fixed.clone();
            rejected.requested_path = path.clone();
            rejected.path = path;
            rejected.base = None;
            rejected.outcome = ProjectResourceOutcome::Failed(failure);
            return rejected;
        }
        let fixed = FixedResource {
            source_id: source_id.clone(),
            requested_path: path.clone(),
            path,
            base: None,
            no_symlinks: false,
            outcome: ProjectResourceOutcome::Failed(failure),
        };
        self.fixed.insert(source_id, fixed.clone());
        fixed
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

    fn current_read_limit(&self) -> ProjectLimit {
        let usage = self.job.usage().unwrap_or_default();
        if usage.read_operations
            >= u64::try_from(self.limits.filesystem_reads.max_files).unwrap_or(u64::MAX)
        {
            ProjectLimit::Files {
                limit: self.limits.filesystem_reads.max_files,
            }
        } else {
            ProjectLimit::ReadBytes {
                limit: self.limits.filesystem_reads.max_total_bytes,
            }
        }
    }

    fn confined_session(
        &mut self,
        roots: &[PathBuf],
    ) -> Result<LocalFilesystemSession, ResourceError> {
        let mut selected = Vec::new();
        for root in roots {
            let anchor = self
                .authority
                .policy_for_path(root)
                .map(|policy| policy.root().to_owned())
                .ok_or_else(|| ResourceError::OutsideRoots(root.clone()))?;
            let access = if anchor == *root {
                self.authority
                    .access_existing([anchor], self.limits.filesystem_reads)?
            } else {
                self.authority.access_derived(
                    &anchor,
                    DerivedFilesystemRoots {
                        confined: vec![root.clone()],
                        independent: Vec::new(),
                    },
                    self.limits.filesystem_reads,
                )?
            };
            selected.extend(access.roots().iter().cloned());
        }
        selected.sort();
        selected.dedup();
        self.authority
            .access_existing(selected, self.limits.filesystem_reads)?
            .session()
    }

    fn source_id_for_path(&self, path: &Path) -> Result<LogicalSourceId, ProjectError> {
        let value = if path.starts_with(&self.project_root) {
            format!("project:{}", identity_path(&self.project_root, path))
        } else {
            let (index, root) = self
                .identity_roots
                .iter()
                .enumerate()
                .filter(|(_, root)| path.starts_with(root))
                .max_by_key(|(_, root)| root.components().count())
                .ok_or_else(|| {
                    ProjectError::Authority(ResourceError::OutsideRoots(path.to_owned()))
                })?;
            format!("authority:{index}:{}", identity_path(root, path))
        };
        self.source_id_for_value(&value)
            .map_err(ProjectError::Authority)
    }

    fn source_id_for_value(&self, value: &str) -> Result<LogicalSourceId, ResourceError> {
        LogicalSourceId::new(if value.is_empty() { "." } else { value })
    }
}

fn read_fixed_from(
    job: &IncludeFilesystemJob,
    fixed_resources: &mut BTreeMap<LogicalSourceId, FixedResource>,
    limits: crate::ProjectLimits,
    source_id: LogicalSourceId,
    path: PathBuf,
    filesystem: &mut LocalFilesystemSession,
) -> FixedResource {
    if let Some(fixed) = fixed_resources.get(&source_id) {
        return cached_for_session(fixed, filesystem, false);
    }
    let outcome = job
        .transaction(filesystem)
        .map_err(ResourceError::from)
        .and_then(|mut transaction| {
            match transaction.read_utf8_within_budget(IncludeFilesystemPathRequest::new(
                source_id.clone(),
                path.clone(),
            )) {
                IncludeFilesystemBudgetedOutcome::Found(found) => {
                    let canonical = found.provenance().canonical_path().to_owned();
                    let source = Arc::<str>::from(found.source());
                    transaction
                        .commit(filesystem)
                        .map_err(ResourceError::from)?;
                    Ok((
                        canonical.clone(),
                        canonical.parent().map(Path::to_owned),
                        ProjectResourceOutcome::Loaded { source },
                    ))
                }
                IncludeFilesystemBudgetedOutcome::NotFound(missing) => {
                    let candidate = missing.watch_candidate().path().to_owned();
                    transaction
                        .commit(filesystem)
                        .map_err(ResourceError::from)?;
                    Ok((candidate, None, ProjectResourceOutcome::Missing))
                }
                IncludeFilesystemBudgetedOutcome::BudgetExhausted { .. } => Ok((
                    path.clone(),
                    None,
                    ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                        current_read_limit(job, limits),
                    )),
                )),
                IncludeFilesystemBudgetedOutcome::Failed(failed) => {
                    Err(ResourceError::from(failed.error().clone()))
                }
            }
        });
    let (path, base, outcome) = outcome.unwrap_or_else(|error| {
        (
            path,
            None,
            ProjectResourceOutcome::Failed(classify_resource_failure(error, limits)),
        )
    });
    let fixed = FixedResource {
        source_id: source_id.clone(),
        requested_path: path.clone(),
        path,
        base,
        no_symlinks: false,
        outcome,
    };
    fixed_resources.insert(source_id, fixed.clone());
    observe_fixed(&fixed);
    fixed
}

fn resolve_lookup_path(
    target: &str,
    lookup_bases: &BTreeMap<String, PathBuf>,
) -> Result<PathBuf, ResourceError> {
    let (key, base) = lookup_bases
        .iter()
        .filter(|(key, _)| target == key.as_str() || target.starts_with(&format!("{key}/")))
        .max_by_key(|(key, _)| key.len())
        .ok_or_else(|| {
            ResourceError::Unverifiable("include target has no verified filesystem base".to_owned())
        })?;
    let relative = target
        .strip_prefix(key)
        .and_then(|value| value.strip_prefix('/'))
        .unwrap_or("");
    absolute_lexical(base, Path::new(relative))
}

fn cached_for_session(
    fixed: &FixedResource,
    filesystem: &LocalFilesystemSession,
    require_no_symlinks: bool,
) -> FixedResource {
    let within_authority = filesystem
        .roots()
        .iter()
        .any(|root| fixed.requested_path.starts_with(root))
        && (!matches!(fixed.outcome, ProjectResourceOutcome::Loaded { .. })
            || filesystem
                .roots()
                .iter()
                .any(|root| fixed.path.starts_with(root)));
    if within_authority && (!require_no_symlinks || fixed.no_symlinks) {
        return fixed.clone();
    }
    let mut rejected = fixed.clone();
    let error = if !within_authority {
        ResourceError::OutsideRoots(fixed.requested_path.clone())
    } else {
        ResourceError::Unverifiable(
            "resource was not acquired with symbolic links forbidden".to_owned(),
        )
    };
    rejected.outcome = ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(error));
    rejected
}

fn current_read_limit(job: &IncludeFilesystemJob, limits: crate::ProjectLimits) -> ProjectLimit {
    let usage = job.usage().unwrap_or_default();
    if usage.read_operations >= u64::try_from(limits.filesystem_reads.max_files).unwrap_or(u64::MAX)
    {
        ProjectLimit::Files {
            limit: limits.filesystem_reads.max_files,
        }
    } else {
        ProjectLimit::ReadBytes {
            limit: limits.filesystem_reads.max_total_bytes,
        }
    }
}

fn target_result(
    source_id: LogicalSourceId,
    path: PathBuf,
    config: Option<ConfigSnapshot>,
    resolved_config: ResolvedProjectConfig,
    resources: Vec<ProjectResourceResult>,
    outcome: Result<PreprocessedAnalysis, ProjectTargetError>,
) -> ProjectTargetResult {
    ProjectTargetResult {
        source_id,
        path,
        config,
        resolved_config,
        resources,
        outcome,
    }
}

fn include_roots(base: &Path, config: &ResolvedProjectConfig) -> Vec<PathBuf> {
    let mut roots = config.resources.roots.clone();
    if !roots.iter().any(|root| root == base) {
        roots.push(base.to_owned());
    }
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

struct TargetBudget {
    limits: adocweave_host::FilesystemReadLimits,
    ids: BTreeSet<LogicalSourceId>,
    bytes: u64,
}

impl TargetBudget {
    fn new(limits: adocweave_host::FilesystemReadLimits) -> Self {
        Self {
            limits,
            ids: BTreeSet::new(),
            bytes: 0,
        }
    }

    fn charge(&mut self, source_id: &LogicalSourceId, bytes: usize) -> Result<(), ProjectLimit> {
        if self.ids.contains(source_id) {
            return Ok(());
        }
        if self.ids.len() >= self.limits.max_files {
            return Err(ProjectLimit::Files {
                limit: self.limits.max_files,
            });
        }
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        if bytes > self.limits.max_resource_bytes
            || self.bytes.saturating_add(bytes) > self.limits.max_total_bytes
        {
            return Err(ProjectLimit::ReadBytes {
                limit: self.limits.max_total_bytes,
            });
        }
        self.ids.insert(source_id.clone());
        self.bytes += bytes;
        Ok(())
    }
}

fn map_prepared_error(error: PreparedAnalysisError) -> ProjectTargetError {
    match error {
        PreparedAnalysisError::ContractMismatch => ProjectTargetError::Read(
            ResourceError::Unverifiable("processing contract mismatch".to_owned()),
        ),
        PreparedAnalysisError::Parse(error) => {
            ProjectTargetError::Analysis(PreprocessedAnalysisError::Parse(error))
        }
        PreparedAnalysisError::Cancelled => {
            ProjectTargetError::Analysis(PreprocessedAnalysisError::Cancelled)
        }
    }
}

fn project_error_resource(error: ProjectError) -> ResourceError {
    match error {
        ProjectError::Authority(error) => error,
        error => ResourceError::Unverifiable(error.to_string()),
    }
}

fn classify_resource_failure(
    error: ResourceError,
    limits: crate::ProjectLimits,
) -> ProjectResourceFailure {
    match &error {
        ResourceError::FileLimit { limit } => {
            ProjectResourceFailure::Limit(ProjectLimit::Files { limit: *limit })
        }
        ResourceError::ByteLimit | ResourceError::ResourceTooLarge(_) => {
            ProjectResourceFailure::Limit(ProjectLimit::ReadBytes {
                limit: limits.filesystem_reads.max_total_bytes,
            })
        }
        ResourceError::Job(FilesystemJobError::Limit(limit)) => {
            ProjectResourceFailure::Limit(map_job_limit(*limit, limits))
        }
        ResourceError::PermissionDenied(_)
        | ResourceError::Read { .. }
        | ResourceError::InvalidUtf8 { .. } => ProjectResourceFailure::Unreadable(error),
        _ => ProjectResourceFailure::Rejected(error),
    }
}

fn map_job_limit(limit: FilesystemJobLimit, limits: crate::ProjectLimits) -> ProjectLimit {
    match limit {
        FilesystemJobLimit::ReadOperations { .. }
        | FilesystemJobLimit::CandidateChanges { .. }
        | FilesystemJobLimit::Sessions { .. } => ProjectLimit::Files {
            limit: limits.filesystem_reads.max_files,
        },
        FilesystemJobLimit::ReadBytes { .. } | FilesystemJobLimit::ReadProbeBytes { .. } => {
            ProjectLimit::ReadBytes {
                limit: limits.filesystem_reads.max_total_bytes,
            }
        }
        FilesystemJobLimit::DirectoryOperations { .. }
        | FilesystemJobLimit::DirectoryEntries { .. }
        | FilesystemJobLimit::DirectoryProbeEntries { .. } => ProjectLimit::DirectoryEntries {
            limit: limits.max_directory_entries,
        },
    }
}

impl ProjectResourceFailure {
    fn error(&self) -> &ResourceError {
        match self {
            Self::Unreadable(error) | Self::Rejected(error) => error,
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
    use std::fs;
    use std::path::PathBuf;

    use adocweave::OutputLimits;
    use adocweave_host::{FilesystemReadLimits, LocalFilesystemPolicy};

    use super::FIXED_OBSERVER;
    use crate::{
        ConfigSelection, ProjectLimits, ProjectOverrides, ProjectRequest, ProjectTarget, process,
    };

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
        let filesystem_reads = FilesystemReadLimits::default();
        let result = process(ProjectRequest {
            project_root: root.clone(),
            targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
            config: ConfigSelection::Disabled,
            overrides: ProjectOverrides {
                include: Some(true),
            },
            authority: LocalFilesystemPolicy::new([root], filesystem_reads)
                .expect("temporary project authority"),
            limits: ProjectLimits {
                filesystem_reads,
                max_directory_entries: 100,
                max_processing_iterations: 10,
                output: OutputLimits::default(),
            },
        })
        .expect("processing succeeds");
        FIXED_OBSERVER.with(|observer| *observer.borrow_mut() = None);

        let source = &result.targets[0]
            .outcome
            .as_ref()
            .expect("analysis succeeds")
            .document
            .source;
        assert_eq!(source.matches("old content").count(), 2);
        assert!(!source.contains("new content"));
    }
}
