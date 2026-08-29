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

use crate::selection::{absolute_lexical, logical_path, select_targets};
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
    project_policy: adocweave_host::LocalTargetPolicy,
    config_selection: ConfigSelection,
    overrides: crate::ProjectOverrides,
    limits: crate::ProjectLimits,
    selectors: Vec<crate::ProjectTarget>,
    authority: LocalFilesystemPolicy,
    filesystem: LocalFilesystemSession,
    job: IncludeFilesystemJob,
    fixed: BTreeMap<LogicalSourceId, FixedResource>,
    configs: BTreeMap<PathBuf, ConfigSnapshot>,
    processing_iterations: u32,
    output_bytes: u64,
    warnings: Vec<ProjectWarning>,
}

#[derive(Clone, Debug)]
struct FixedResource {
    source_id: LogicalSourceId,
    path: PathBuf,
    base: Option<PathBuf>,
    outcome: ProjectResourceOutcome,
}

struct TargetAnalysisContext<'target> {
    source_id: &'target LogicalSourceId,
    source: &'target Arc<str>,
    config: &'target ResolvedProjectConfig,
    allowed_roots: &'target [PathBuf],
    budget: &'target mut TargetBudget,
    bases: &'target mut BTreeMap<String, PathBuf>,
    resources: &'target mut Vec<ProjectResourceResult>,
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
        authority = authority
            .access_existing(retained_roots, limits.filesystem_reads)
            .map_err(ProjectError::Authority)?;
        let filesystem = authority.session().map_err(ProjectError::Authority)?;
        let read_operations = u64::try_from(limits.filesystem_reads.max_files).unwrap_or(u64::MAX);
        // One scan session per selector, one common read session and at most one
        // confined local-target session per selected document.
        let max_sessions = targets.len().saturating_mul(2).saturating_add(1);
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
            project_policy,
            config_selection: config,
            overrides,
            limits,
            selectors: targets,
            authority,
            filesystem,
            job,
            fixed: BTreeMap::new(),
            configs: BTreeMap::new(),
            processing_iterations: 0,
            output_bytes: 0,
            warnings: Vec::new(),
        })
    }

    fn run(mut self) -> ProjectOutcome {
        let selectors = self.selectors.clone();
        let paths = select_targets(
            &self.project_root,
            &selectors,
            &mut self.authority,
            self.limits,
            &self.job,
            &mut self.warnings,
        )?;
        let mut targets = Vec::with_capacity(paths.len());
        for path in paths {
            targets.push(self.process_target(path)?);
        }
        let processing_iterations = self.processing_iterations;
        let output_bytes = self.output_bytes;
        let warnings = self.warnings;
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
        let (config_snapshot, mut config, config_resource) = self.resolve_config(&path)?;
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
                return Ok(target_result(
                    source_id,
                    path,
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Incomplete(*limit)),
                ));
            }
            ProjectResourceOutcome::Missing => {
                return Ok(target_result(
                    source_id,
                    path.clone(),
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Read(ResourceError::Missing(path))),
                ));
            }
            ProjectResourceOutcome::Failed(failure) => {
                return Ok(target_result(
                    source_id,
                    path,
                    config_snapshot,
                    config,
                    resources,
                    Err(ProjectTargetError::Read(failure.error().clone())),
                ));
            }
            ProjectResourceOutcome::Present => unreachable!("a primary document is read"),
        };
        let mut target_budget = TargetBudget::new(config.resources.limit_plan.filesystem_reads);
        if let Err(limit) = target_budget.charge(&source_id, source.len()) {
            return Ok(target_result(
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
        let allowed_roots = include_roots(&path, &config);
        let mut outcome = self.analyze_target(TargetAnalysisContext {
            source_id: &source_id,
            source: &source,
            config: &config,
            allowed_roots: &allowed_roots,
            budget: &mut target_budget,
            bases: &mut bases,
            resources: &mut resources,
        });
        if let Ok(analysis) = &outcome {
            let bytes = u64::try_from(analysis.document.source.len()).unwrap_or(u64::MAX);
            let total = self.output_bytes.saturating_add(bytes);
            if total > u64::from(self.limits.output.max_output_bytes) {
                outcome = Err(ProjectTargetError::Incomplete(ProjectLimit::OutputBytes {
                    limit: self.limits.output.max_output_bytes,
                }));
            } else {
                self.output_bytes = total;
            }
        }
        if let Ok(analysis) = &outcome {
            self.collect_local_targets(analysis, &config, &bases, &mut resources);
        }
        self.collect_stylesheets(&source_id, &config, &mut resources);
        resources.sort_by(|left, right| {
            left.source_id
                .cmp(&right.source_id)
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| resource_kind_order(left.kind).cmp(&resource_kind_order(right.kind)))
        });
        Ok(target_result(
            source_id,
            path,
            config_snapshot,
            config,
            resources,
            outcome,
        ))
    }

    fn resolve_config(
        &mut self,
        target: &Path,
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
            ConfigSelection::Discover => {
                adocweave_config::discover_with_policy(target, &self.project_policy)
                    .map_err(ProjectError::Config)?
            }
        };
        let Some(path) = path else {
            return Ok((None, ResolvedProjectConfig::default(), None));
        };
        let snapshot = self.load_config(path.clone())?;
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
        if let Some(fixed) = self.fixed.get(&source_id) {
            return match &fixed.outcome {
                ProjectResourceOutcome::Loaded { .. } => {
                    Err(ProjectError::Authority(ResourceError::Unverifiable(
                        "configuration identity already names another resource".to_owned(),
                    )))
                }
                ProjectResourceOutcome::Missing => {
                    Err(ProjectError::Authority(ResourceError::Missing(path)))
                }
                ProjectResourceOutcome::Failed(failure) => {
                    Err(ProjectError::Authority(failure.error().clone()))
                }
                ProjectResourceOutcome::Present => {
                    Err(ProjectError::Authority(ResourceError::Unverifiable(
                        "configuration identity was only inspected".to_owned(),
                    )))
                }
            };
        }
        let mut transaction = self
            .job
            .transaction(&self.filesystem)
            .map_err(|error| ProjectError::Authority(ResourceError::from(error)))?;
        let outcome = transaction.read_utf8_no_symlinks_within_budget(
            IncludeFilesystemPathRequest::new(source_id.clone(), path.clone()),
        );
        match outcome {
            IncludeFilesystemBudgetedOutcome::Found(found) => {
                let snapshot = ConfigSnapshot::from_include_filesystem_source(&found)
                    .map_err(ProjectError::Config)?;
                let canonical = found.provenance().canonical_path().to_owned();
                let source = Arc::<str>::from(found.source());
                transaction
                    .commit(&mut self.filesystem)
                    .map_err(|error| ProjectError::Authority(ResourceError::from(error)))?;
                self.fixed.insert(
                    source_id.clone(),
                    FixedResource {
                        source_id,
                        base: canonical.parent().map(Path::to_owned),
                        path: canonical,
                        outcome: ProjectResourceOutcome::Loaded { source },
                    },
                );
                self.configs.insert(path, snapshot.clone());
                Ok(snapshot)
            }
            IncludeFilesystemBudgetedOutcome::NotFound(missing) => {
                transaction
                    .commit(&mut self.filesystem)
                    .map_err(|error| ProjectError::Authority(ResourceError::from(error)))?;
                Err(ProjectError::Authority(ResourceError::Missing(
                    missing.watch_candidate().path().to_owned(),
                )))
            }
            IncludeFilesystemBudgetedOutcome::BudgetExhausted { .. } => {
                Err(ProjectError::Limit(self.current_read_limit()))
            }
            IncludeFilesystemBudgetedOutcome::Failed(failed) => Err(ProjectError::Authority(
                ResourceError::from(failed.error().clone()),
            )),
        }
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
            resources,
        } = context;
        let mut preprocess = config.preprocess.clone();
        preprocess.enable_includes = config.resources.include;
        preprocess.source_id = Some(SourceId::new(source_id.as_str()));
        let base = bases
            .get(source_id.as_str())
            .expect("the primary source has a verified base");
        let base_key = logical_path(&self.project_root, base);
        preprocess.base_uri = (!base_key.is_empty()).then_some(base_key);
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
                    let include_id = self
                        .source_id_for_value(&target)
                        .map_err(ProjectTargetError::Read)?;
                    let path = absolute_lexical(&self.project_root, Path::new(&target))
                        .map_err(ProjectTargetError::Read)?;
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
                    let fixed = self.read_fixed(include_id.clone(), path);
                    resources
                        .push(fixed.result(ProjectResourceKind::Include, requested_by.clone()));
                    let response = match &fixed.outcome {
                        ProjectResourceOutcome::Loaded { source } => {
                            budget
                                .charge(&include_id, source.len())
                                .map_err(ProjectTargetError::Incomplete)?;
                            if let Some(base) = &fixed.base {
                                bases.insert(include_id.as_str().to_owned(), base.clone());
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
                        ProjectResourceOutcome::Failed(ProjectResourceFailure::Unreadable(
                            error,
                        )) => return Err(ProjectTargetError::Read(error.clone())),
                        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(error)) => {
                            return Err(ProjectTargetError::Read(error.clone()));
                        }
                        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(limit)) => {
                            return Err(ProjectTargetError::Incomplete(*limit));
                        }
                        ProjectResourceOutcome::Present => unreachable!("an include is read"),
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
            let Ok(source_id) = self.source_id_for_path(path) else {
                continue;
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
        let Ok(local_policy) = self.authority.access_derived(
            &self.project_root,
            DerivedFilesystemRoots {
                confined: vec![authority.to_owned()],
                independent: Vec::new(),
            },
            self.limits.filesystem_reads,
        ) else {
            return;
        };
        let Ok(mut local_filesystem) = local_policy.session() else {
            return;
        };
        let Ok(projection) = analysis.project_origins(ProjectionLimits::default()) else {
            return;
        };
        let candidates = projection
            .local_targets
            .into_iter()
            .filter_map(|target| {
                let owner = target
                    .origins
                    .first()
                    .and_then(|origin| origin.source_id.as_ref())
                    .map_or_else(
                        || analysis.analysis.source_id().map(SourceId::as_str),
                        |source| Some(source.as_str()),
                    )?;
                let base = bases.get(owner)?.clone();
                Some((owner.to_owned(), base, target.value))
            })
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for (owner, base, target) in candidates {
            if !seen.insert((owner.clone(), target.path.clone())) {
                continue;
            }
            let requested_by = self.source_id_for_value(&owner).ok();
            let path = match absolute_lexical(&base, Path::new(&target.path)) {
                Ok(path) => path,
                Err(error) => {
                    if let Ok(source_id) = self.source_id_for_value(&target.path) {
                        let fixed = self.fix_failure(
                            source_id,
                            base.join(&target.path),
                            ProjectResourceFailure::Rejected(error),
                        );
                        resources
                            .push(fixed.result(ProjectResourceKind::LocalTarget, requested_by));
                    }
                    continue;
                }
            };
            let Ok(source_id) = self.source_id_for_path(&path) else {
                continue;
            };
            let fixed = if target.syntax == adocweave::LocalTargetSyntax::Unverifiable {
                self.fix_failure(
                    source_id,
                    path,
                    ProjectResourceFailure::Rejected(ResourceError::Unverifiable(target.target)),
                )
            } else {
                self.inspect_fixed_in(
                    source_id,
                    path,
                    authority,
                    &base,
                    &target.path,
                    &mut local_filesystem,
                )
            };
            resources.push(fixed.result(ProjectResourceKind::LocalTarget, requested_by));
        }
    }

    fn read_fixed(&mut self, source_id: LogicalSourceId, path: PathBuf) -> FixedResource {
        if let Some(fixed) = self.fixed.get(&source_id) {
            return fixed.clone();
        }
        let outcome = self
            .job
            .transaction(&self.filesystem)
            .map_err(ResourceError::from)
            .and_then(|mut transaction| {
                let outcome = transaction.read_utf8_within_budget(
                    IncludeFilesystemPathRequest::new(source_id.clone(), path.clone()),
                );
                match outcome {
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
                        let candidate = missing.watch_candidate().path().to_owned();
                        transaction
                            .commit(&mut self.filesystem)
                            .map_err(ResourceError::from)?;
                        Ok((candidate, None, ProjectResourceOutcome::Missing))
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
        let (path, base, outcome) = match outcome {
            Ok(value) => value,
            Err(error) => (
                path,
                None,
                ProjectResourceOutcome::Failed(classify_resource_failure(error, self.limits)),
            ),
        };
        let fixed = FixedResource {
            source_id: source_id.clone(),
            path,
            base,
            outcome,
        };
        self.fixed.insert(source_id, fixed.clone());
        observe_fixed(&fixed);
        fixed
    }

    fn inspect_fixed_in(
        &mut self,
        source_id: LogicalSourceId,
        path: PathBuf,
        authority: &Path,
        base: &Path,
        target: &str,
        filesystem: &mut LocalFilesystemSession,
    ) -> FixedResource {
        if let Some(fixed) = self.fixed.get(&source_id) {
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
        let fixed = FixedResource {
            source_id: source_id.clone(),
            path,
            base: None,
            outcome,
        };
        self.fixed.insert(source_id, fixed.clone());
        observe_fixed(&fixed);
        fixed
    }

    fn fix_failure(
        &mut self,
        source_id: LogicalSourceId,
        path: PathBuf,
        failure: ProjectResourceFailure,
    ) -> FixedResource {
        if let Some(fixed) = self.fixed.get(&source_id) {
            return fixed.clone();
        }
        let fixed = FixedResource {
            source_id: source_id.clone(),
            path,
            base: None,
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

    fn source_id_for_path(&self, path: &Path) -> Result<LogicalSourceId, ProjectError> {
        self.source_id_for_value(&logical_path(&self.project_root, path))
            .map_err(ProjectError::Authority)
    }

    fn source_id_for_value(&self, value: &str) -> Result<LogicalSourceId, ResourceError> {
        LogicalSourceId::new(if value.is_empty() { "." } else { value })
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

fn include_roots(path: &Path, config: &ResolvedProjectConfig) -> Vec<PathBuf> {
    let mut roots = config.resources.roots.clone();
    if let Some(parent) = path.parent()
        && !roots.iter().any(|root| root == parent)
    {
        roots.push(parent.to_owned());
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
