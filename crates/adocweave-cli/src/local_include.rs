//! Explicit, bounded local resource provider owned by the CLI binary.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use adocweave::preprocess::{EffectiveProcessingOptions, PreprocessOptions, PreprocessedDocument};
#[cfg(test)]
use adocweave_host::{FilesystemReadLimits, LocalFilesystemPolicy, LocalTargetPolicy};
use adocweave_host::{
    IncludeFilesystem, IncludeFilesystemOutcome, IncludeFilesystemRequest, LocalFilesystemSession,
    LocalTargetError, LogicalSourceId, ResourceError,
};
use adocweave_workspace::{
    NeverCancelled, ResourceId, Revision, Workspace, WorkspaceIncludeResolution, WorkspaceLimits,
    WorkspacePreprocessDraft, WorkspacePreprocessOutcome, WorkspaceResourceLoad,
    WorkspaceResourceLoadEvent, WorkspaceResourceLoader, WorkspaceResourceRequest,
    WorkspaceResourceResolution,
};

#[derive(Debug)]
pub enum LocalIncludeError {
    #[cfg(test)]
    InvalidBase {
        path: PathBuf,
        source: std::io::Error,
    },
    #[cfg(test)]
    InvalidRoot {
        path: PathBuf,
        source: std::io::Error,
    },
    OutsideRoot(PathBuf),
    Analysis(String),
    Host(ResourceError),
}

pub struct PreparedInput {
    projection: ProjectionInput,
    validation: Option<LocalValidationContext>,
    dependencies: DependencyJournal,
}

#[derive(Debug)]
pub(crate) struct PrepareFailure {
    error: Box<LocalIncludeError>,
    dependencies: DependencyJournal,
}

impl PrepareFailure {
    pub(crate) fn into_error(self) -> LocalIncludeError {
        *self.error
    }

    pub(crate) fn dependency_entries(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
        self.dependencies.entries()
    }

    fn with_dependencies(error: LocalIncludeError, dependencies: DependencyJournal) -> Self {
        Self {
            error: Box::new(error),
            dependencies,
        }
    }
}

impl From<LocalIncludeError> for PrepareFailure {
    fn from(error: LocalIncludeError) -> Self {
        Self::with_dependencies(error, DependencyJournal::default())
    }
}

pub struct ProjectionInput {
    draft: WorkspacePreprocessDraft,
    source_keys: BTreeMap<String, ResourceId>,
}

pub struct LocalValidationContext {
    include_errors: Vec<IncludeFailure>,
}

#[derive(Clone, Debug)]
struct IncludeFailure {
    target: String,
    error: LocalTargetError,
}

#[derive(Clone, Debug, Default)]
struct DependencyJournal {
    entries: BTreeMap<PathBuf, Option<Arc<str>>>,
}

impl DependencyJournal {
    fn entries(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
        self.entries
            .iter()
            .map(|(path, source)| (path.as_path(), source.as_deref()))
    }

    fn observe_candidate(&mut self, path: &Path) {
        self.entries.entry(path.to_owned()).or_default();
    }

    fn observe_loaded(&mut self, path: &Path, source: Arc<str>) {
        self.entries.insert(path.to_owned(), Some(source));
    }
}

impl PreparedInput {
    pub fn projection(&self) -> &ProjectionInput {
        &self.projection
    }

    pub fn validation(&self) -> Option<&LocalValidationContext> {
        self.validation.as_ref()
    }

    pub(crate) fn resource_sizes(&self) -> impl Iterator<Item = u64> + '_ {
        self.projection.resource_lengths()
    }

    pub(crate) fn dependency_entries(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
        self.dependencies.entries()
    }
}

impl ProjectionInput {
    pub fn document(&self) -> &PreprocessedDocument {
        self.draft.document()
    }

    pub fn resource_lengths(&self) -> impl Iterator<Item = u64> + '_ {
        self.source_keys.values().filter_map(|key| {
            self.draft
                .source(key)
                .map(|source| u64::try_from(source.len()).unwrap_or(u64::MAX))
        })
    }
}

impl LocalValidationContext {
    pub(crate) fn include_errors(&self) -> impl Iterator<Item = (&str, &LocalTargetError)> {
        self.include_errors
            .iter()
            .map(|failure| (failure.target.as_str(), &failure.error))
    }
}

pub(crate) fn include_target_error(error: ResourceError) -> LocalTargetError {
    match error {
        ResourceError::Missing(path) => LocalTargetError::Missing(path),
        ResourceError::PermissionDenied(path) => LocalTargetError::PermissionDenied(path),
        ResourceError::OutsideRoots(path) => LocalTargetError::OutsideRoot(path),
        ResourceError::NotRegularFile(path) => LocalTargetError::NotFile(path),
        ResourceError::InvalidUtf8 { path, .. } => LocalTargetError::InvalidUtf8(path),
        ResourceError::ResourceTooLarge(path) => LocalTargetError::ResourceTooLarge(path),
        ResourceError::FileLimit { limit } => LocalTargetError::LimitExceeded { limit },
        ResourceError::ByteLimit => LocalTargetError::ReadLimitExceeded,
        ResourceError::Unverifiable(reason) => LocalTargetError::Unverifiable(reason),
        other => LocalTargetError::Unverifiable(other.to_string()),
    }
}

pub(crate) fn read_utf8_with_session(
    filesystem: &mut LocalFilesystemSession,
    source_id: impl Into<String>,
    path: &Path,
) -> Result<Arc<str>, LocalIncludeError> {
    let source_id = LogicalSourceId::new(source_id.into()).map_err(LocalIncludeError::Host)?;
    match IncludeFilesystem::new().read_utf8(
        filesystem,
        adocweave_host::IncludeFilesystemPathRequest::new(source_id, path),
    ) {
        IncludeFilesystemOutcome::Found(found) => {
            let (_, source, _) = found.into_parts();
            Ok(source)
        }
        IncludeFilesystemOutcome::NotFound(missing) => Err(LocalIncludeError::Host(
            ResourceError::Missing(missing.watch_candidate().path().to_owned()),
        )),
        IncludeFilesystemOutcome::Failed(failed) => Err(LocalIncludeError::Host(
            ResourceError::from(failed.error().clone()),
        )),
    }
}

impl fmt::Display for LocalIncludeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(test)]
            Self::InvalidBase { path, source } => {
                write!(
                    formatter,
                    "invalid include base {}: {source}",
                    path.display()
                )
            }
            #[cfg(test)]
            Self::InvalidRoot { path, source } => {
                write!(
                    formatter,
                    "invalid include root {}: {source}",
                    path.display()
                )
            }
            Self::OutsideRoot(path) => {
                write!(
                    formatter,
                    "include target is outside allowed roots: {}",
                    path.display()
                )
            }
            Self::Analysis(error) => formatter.write_str(error),
            Self::Host(error) => error.fmt(formatter),
        }
    }
}

impl Error for LocalIncludeError {}

enum IncludeReadMode {
    #[cfg(test)]
    General {
        base: PathBuf,
        base_policy: LocalTargetPolicy,
        allowed: Vec<LocalTargetPolicy>,
    },
    Local {
        root: PathBuf,
    },
}

impl IncludeReadMode {
    fn watch_candidate(&self, target: &str) -> Option<PathBuf> {
        match self {
            Self::Local { root } => Some(root.join(target)),
            #[cfg(test)]
            Self::General { .. } => None,
        }
    }

    fn read(
        &self,
        filesystem: &mut LocalFilesystemSession,
        source_id: LogicalSourceId,
        target: &str,
    ) -> Result<IncludeFilesystemOutcome, LocalIncludeError> {
        let provider = IncludeFilesystem::new();
        match self {
            #[cfg(test)]
            Self::General {
                base,
                base_policy,
                allowed,
            } => {
                let candidate = base.join(target);
                let path = if allowed.is_empty() {
                    base_policy.normalize_candidate(&candidate)
                } else {
                    allowed
                        .iter()
                        .find_map(|policy| policy.normalize_candidate(&candidate).ok())
                        .ok_or_else(|| LocalTargetError::OutsideRoot(candidate.clone()))
                };
                match path {
                    Ok(path) => Ok(provider.read_utf8(
                        filesystem,
                        adocweave_host::IncludeFilesystemPathRequest::new(source_id, path),
                    )),
                    Err(_) => Err(LocalIncludeError::OutsideRoot(candidate)),
                }
            }
            Self::Local { root } => Ok(provider.read(
                filesystem,
                IncludeFilesystemRequest::new(source_id, root, target),
            )),
        }
    }
}

struct LocalIncludeLoader<'session> {
    read_mode: IncludeReadMode,
    filesystem: &'session mut LocalFilesystemSession,
    validate_local_targets: bool,
}

#[derive(Default)]
struct LocalIncludeEvidence {
    watch_candidates: Vec<PathBuf>,
    loaded: Option<LoadedIncludeEvidence>,
    failure: Option<LocalTargetError>,
}

struct LoadedIncludeEvidence {
    source_id: String,
    source: Arc<str>,
    canonical_path: PathBuf,
}

struct CollectedIncludeEvidence {
    dependencies: DependencyJournal,
    source_keys: BTreeMap<String, ResourceId>,
    failure_errors: BTreeMap<String, LocalTargetError>,
}

fn collect_include_evidence(
    source_id: String,
    root_id: ResourceId,
    _source_base: PathBuf,
    _include_base: Option<PathBuf>,
    _validate_local_targets: bool,
    loads: Vec<WorkspaceResourceLoadEvent<LocalIncludeEvidence>>,
) -> CollectedIncludeEvidence {
    let mut collected = CollectedIncludeEvidence {
        dependencies: DependencyJournal::default(),
        source_keys: BTreeMap::from([(source_id.clone(), root_id)]),
        failure_errors: BTreeMap::new(),
    };
    for event in loads {
        let (request, evidence) = event.into_parts();
        for candidate in evidence.watch_candidates {
            collected.dependencies.observe_candidate(&candidate);
        }
        if let Some(loaded) = evidence.loaded {
            collected
                .dependencies
                .observe_loaded(&loaded.canonical_path, Arc::clone(&loaded.source));
            let key = ResourceId::new(request.target())
                .expect("workspace driver returned a valid resource target");
            collected.source_keys.insert(loaded.source_id.clone(), key);
        }
        if let Some(error) = evidence.failure {
            collected
                .failure_errors
                .insert(request.target().to_owned(), error);
        }
    }
    collected
}

impl WorkspaceResourceLoader for LocalIncludeLoader<'_> {
    type Error = LocalIncludeError;
    type Evidence = LocalIncludeEvidence;

    fn load(
        &mut self,
        request: &WorkspaceResourceRequest,
    ) -> WorkspaceResourceLoad<Self::Evidence, Self::Error> {
        let target = request.target().to_owned();
        let projected_source_id = include_source_id(&target);
        let logical_id = match LogicalSourceId::new(projected_source_id.clone()) {
            Ok(logical_id) => logical_id,
            Err(error) => {
                return WorkspaceResourceLoad::failed(
                    LocalIncludeError::Host(error),
                    LocalIncludeEvidence::default(),
                );
            }
        };
        let request_range = request.range();
        let inspect = !self.validate_local_targets
            || adocweave::LocalTargetReference::from_include(
                request_range,
                request_range,
                request.authored_target(),
            )
            .is_some_and(|reference| reference.syntax == adocweave::LocalTargetSyntax::Candidate);
        if !inspect {
            return WorkspaceResourceLoad::resolved(
                WorkspaceResourceResolution::FailedWithPlaceholder {
                    source_id: projected_source_id,
                },
                LocalIncludeEvidence {
                    failure: Some(LocalTargetError::Unverifiable(target)),
                    ..LocalIncludeEvidence::default()
                },
            );
        }
        let mut evidence = LocalIncludeEvidence::default();
        if let Some(candidate) = self.read_mode.watch_candidate(&target) {
            evidence.watch_candidates.push(candidate);
        }
        let outcome = match self.read_mode.read(self.filesystem, logical_id, &target) {
            Ok(outcome) => outcome,
            Err(error) => return WorkspaceResourceLoad::failed(error, evidence),
        };
        match outcome {
            IncludeFilesystemOutcome::Found(found) => {
                let source = Arc::<str>::from(found.source());
                for candidate in found.watch_candidates() {
                    evidence.watch_candidates.push(candidate.path().to_owned());
                }
                evidence.loaded = Some(LoadedIncludeEvidence {
                    source_id: projected_source_id.clone(),
                    source: Arc::clone(&source),
                    canonical_path: found.provenance().canonical_path().to_owned(),
                });
                WorkspaceResourceLoad::resolved(
                    WorkspaceResourceResolution::Found {
                        source_id: projected_source_id,
                        source,
                    },
                    evidence,
                )
            }
            IncludeFilesystemOutcome::NotFound(missing) => {
                let candidate = missing.watch_candidate().path().to_owned();
                evidence.watch_candidates.push(candidate.clone());
                if !self.validate_local_targets {
                    return WorkspaceResourceLoad::failed(
                        LocalIncludeError::Host(ResourceError::Missing(candidate)),
                        evidence,
                    );
                }
                evidence.failure = Some(LocalTargetError::Missing(candidate));
                WorkspaceResourceLoad::resolved(
                    WorkspaceResourceResolution::FailedWithPlaceholder {
                        source_id: projected_source_id,
                    },
                    evidence,
                )
            }
            IncludeFilesystemOutcome::Failed(failed) => {
                let host_error = ResourceError::from(failed.error().clone());
                if !self.validate_local_targets {
                    return WorkspaceResourceLoad::failed(
                        LocalIncludeError::Host(host_error),
                        evidence,
                    );
                }
                evidence.failure = Some(include_target_error(host_error));
                WorkspaceResourceLoad::resolved(
                    WorkspaceResourceResolution::FailedWithPlaceholder {
                        source_id: projected_source_id,
                    },
                    evidence,
                )
            }
        }
    }
}

/// Authority-checked input ready for the shared workspace driver.
struct ResolvedPrepareRequest<'request> {
    source: &'request str,
    source_id: String,
    source_base: PathBuf,
    include_base: Option<PathBuf>,
    preprocess_options: PreprocessOptions,
    analysis_options: &'request adocweave::AnalysisOptions,
    read_mode: IncludeReadMode,
    validate_local_targets: bool,
}

fn prepare_with_driver(
    request: ResolvedPrepareRequest<'_>,
    filesystem: &mut LocalFilesystemSession,
) -> Result<PreparedInput, PrepareFailure> {
    let ResolvedPrepareRequest {
        source,
        source_id,
        source_base,
        include_base,
        mut preprocess_options,
        analysis_options,
        read_mode,
        validate_local_targets,
    } = request;
    let root_id = ResourceId::new(source_id.clone())
        .map_err(|error| LocalIncludeError::Analysis(error.to_string()))?;
    let mut workspace = Workspace::new(WorkspaceLimits::default());
    workspace
        .upsert_disk(root_id.clone(), Revision::new(1), Arc::<str>::from(source))
        .and_then(|_| workspace.register_root(root_id.clone()))
        .map_err(|error| LocalIncludeError::Analysis(error.to_string()))?;
    preprocess_options.enable_includes = true;
    let options = EffectiveProcessingOptions::new(analysis_options.clone(), preprocess_options)
        .map_err(|error| LocalIncludeError::Analysis(error.to_string()))?;
    let snapshot = workspace.snapshot();
    let mut loader = LocalIncludeLoader {
        read_mode,
        filesystem,
        validate_local_targets,
    };
    let (outcome, loads) = snapshot
        .preprocess_with(&root_id, &options, &mut loader, &NeverCancelled)
        .into_parts();
    let CollectedIncludeEvidence {
        dependencies,
        source_keys,
        failure_errors,
    } = collect_include_evidence(
        source_id,
        root_id,
        source_base,
        include_base,
        validate_local_targets,
        loads,
    );
    let draft = match outcome {
        WorkspacePreprocessOutcome::Complete(draft) => *draft,
        WorkspacePreprocessOutcome::LoaderFailed(error) => {
            return Err(PrepareFailure::with_dependencies(error, dependencies));
        }
        WorkspacePreprocessOutcome::CoreFailed(failure) => {
            return Err(PrepareFailure::with_dependencies(
                LocalIncludeError::Analysis(failure.error().to_string()),
                dependencies,
            ));
        }
        WorkspacePreprocessOutcome::Cancelled => {
            return Err(PrepareFailure::with_dependencies(
                LocalIncludeError::Analysis("include preprocessing was cancelled".to_owned()),
                dependencies,
            ));
        }
    };
    let include_errors = draft
        .include_journal()
        .iter()
        .filter(|event| event.resolution() == WorkspaceIncludeResolution::Failed)
        .filter_map(|event| {
            failure_errors
                .get(event.target().as_str())
                .cloned()
                .map(|error| IncludeFailure {
                    target: event.target().to_string(),
                    error,
                })
        })
        .collect();
    let validation = validate_local_targets.then_some(LocalValidationContext { include_errors });
    Ok(PreparedInput {
        projection: ProjectionInput { draft, source_keys },
        validation,
        dependencies,
    })
}

pub(crate) struct PrepareRequest<'request> {
    source: &'request str,
    source_id: String,
    preprocess_options: &'request PreprocessOptions,
    analysis_options: &'request adocweave::AnalysisOptions,
    authority: PrepareAuthority,
}

enum PrepareAuthority {
    #[cfg(test)]
    General {
        base_dir: PathBuf,
        allowed_roots: Vec<PathBuf>,
    },
    Project {
        base_dir: PathBuf,
        source_base: PathBuf,
        project_root: PathBuf,
    },
}

impl<'request> PrepareRequest<'request> {
    #[cfg(test)]
    pub(crate) fn general(
        source: &'request str,
        source_id: String,
        base_dir: &'request Path,
        allowed_roots: &'request [PathBuf],
        preprocess_options: &'request PreprocessOptions,
        analysis_options: &'request adocweave::AnalysisOptions,
    ) -> Self {
        Self {
            source,
            source_id,
            preprocess_options,
            analysis_options,
            authority: PrepareAuthority::General {
                base_dir: base_dir.to_owned(),
                allowed_roots: allowed_roots.to_vec(),
            },
        }
    }

    pub(crate) fn project(
        source: &'request str,
        source_id: String,
        base_dir: &'request Path,
        source_base: &'request Path,
        project_root: &'request Path,
        preprocess_options: &'request PreprocessOptions,
        analysis_options: &'request adocweave::AnalysisOptions,
    ) -> Self {
        Self {
            source,
            source_id,
            preprocess_options,
            analysis_options,
            authority: PrepareAuthority::Project {
                base_dir: base_dir.to_owned(),
                source_base: source_base.to_owned(),
                project_root: project_root.to_owned(),
            },
        }
    }

    #[cfg(test)]
    fn canonicalize_general_authority(mut self) -> Result<Self, LocalIncludeError> {
        if let PrepareAuthority::General {
            base_dir,
            allowed_roots,
        } = &mut self.authority
        {
            *base_dir =
                base_dir
                    .canonicalize()
                    .map_err(|source| LocalIncludeError::InvalidBase {
                        path: base_dir.clone(),
                        source,
                    })?;
            *allowed_roots = allowed_roots
                .iter()
                .map(|path| {
                    path.canonicalize()
                        .map_err(|source| LocalIncludeError::InvalidRoot {
                            path: path.clone(),
                            source,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(self)
    }

    #[cfg(test)]
    fn filesystem_policy(
        &self,
        limits: FilesystemReadLimits,
    ) -> Result<LocalFilesystemPolicy, LocalIncludeError> {
        let roots = match &self.authority {
            PrepareAuthority::General {
                base_dir,
                allowed_roots,
            } => {
                let mut roots = allowed_roots.clone();
                if !roots.contains(base_dir) {
                    roots.push(base_dir.clone());
                }
                roots
            }
            PrepareAuthority::Project { project_root, .. } => vec![project_root.clone()],
        };
        LocalFilesystemPolicy::new(roots, limits).map_err(LocalIncludeError::Host)
    }
}

#[cfg(test)]
pub(crate) fn prepare(
    request: PrepareRequest<'_>,
    limits: FilesystemReadLimits,
) -> Result<PreparedInput, PrepareFailure> {
    let request = request.canonicalize_general_authority()?;
    let policy = request.filesystem_policy(limits)?;
    let mut filesystem = policy.session().map_err(LocalIncludeError::Host)?;
    prepare_with_session(request, &mut filesystem)
}

pub(crate) fn prepare_with_session(
    request: PrepareRequest<'_>,
    filesystem: &mut LocalFilesystemSession,
) -> Result<PreparedInput, PrepareFailure> {
    let PrepareRequest {
        source,
        source_id,
        preprocess_options,
        analysis_options,
        authority,
    } = request;
    let (source_base, include_base, preprocess_options, read_mode, validate_local_targets) =
        match authority {
            #[cfg(test)]
            PrepareAuthority::General {
                base_dir,
                allowed_roots,
            } => {
                let base_policy = filesystem
                    .policy_for_path(&base_dir)
                    .ok_or_else(|| LocalIncludeError::OutsideRoot(base_dir.to_owned()))?
                    .derive_confined_directory(&base_dir)
                    .map_err(|error| {
                        LocalIncludeError::Analysis(format!("invalid include base: {error}"))
                    })?;
                let base_dir = base_policy.root().to_owned();
                let allowed = allowed_roots
                    .iter()
                    .map(|path| {
                        filesystem
                            .policy_for_path(path)
                            .ok_or_else(|| LocalIncludeError::OutsideRoot(path.clone()))?
                            .derive_confined_directory(path)
                            .map_err(|error| {
                                LocalIncludeError::Analysis(format!(
                                    "invalid include root: {error}"
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                (
                    base_dir.clone(),
                    None,
                    preprocess_options.clone(),
                    IncludeReadMode::General {
                        base: base_dir,
                        base_policy,
                        allowed,
                    },
                    false,
                )
            }
            PrepareAuthority::Project {
                base_dir,
                source_base,
                project_root,
            } => {
                let policy = filesystem
                    .policy_for_path(&project_root)
                    .ok_or_else(|| LocalIncludeError::OutsideRoot(project_root.to_owned()))?
                    .derive_confined_directory(&project_root)
                    .map_err(|error| {
                        LocalIncludeError::Analysis(format!("invalid project root: {error}"))
                    })?;
                let base_dir =
                    policy
                        .inspect_directory_no_symlinks(&base_dir)
                        .map_err(|error| {
                            LocalIncludeError::Analysis(format!("invalid include base: {error}"))
                        })?;
                let root = policy.root().to_owned();
                let base_key = logical_key(
                    base_dir
                        .strip_prefix(&root)
                        .expect("base checked below root"),
                );
                let source_base =
                    policy
                        .inspect_directory_no_symlinks(&source_base)
                        .map_err(|error| {
                            LocalIncludeError::Analysis(format!("invalid source base: {error}"))
                        })?;
                let mut options = preprocess_options.clone();
                options.base_uri = (!base_key.is_empty()).then_some(base_key);
                (
                    source_base,
                    Some(base_dir),
                    options,
                    IncludeReadMode::Local { root },
                    true,
                )
            }
        };
    prepare_with_driver(
        ResolvedPrepareRequest {
            source,
            source_id,
            source_base,
            include_base,
            preprocess_options,
            analysis_options,
            read_mode,
            validate_local_targets,
        },
        filesystem,
    )
}

fn logical_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn include_source_id(logical_target: &str) -> String {
    format!("include:{logical_target}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "adocweave-include-loader-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("test directory cleanup");
        }
    }

    fn session(root: &Path) -> LocalFilesystemSession {
        LocalFilesystemPolicy::new([root.to_owned()], FilesystemReadLimits::default())
            .and_then(|policy| policy.session())
            .expect("session")
    }

    #[test]
    fn duplicate_requests_keep_logical_identity_and_share_one_read() {
        let root = TestDirectory::new();
        fs::write(root.0.join("part.adoc"), "part\n").expect("fixture");
        let mut session = session(&root.0);

        for _ in 0..2 {
            let loaded = IncludeFilesystem::new().read(
                &mut session,
                IncludeFilesystemRequest::new(
                    LogicalSourceId::new("include:part.adoc").expect("source ID"),
                    &root.0,
                    "part.adoc",
                ),
            );
            let IncludeFilesystemOutcome::Found(loaded) = loaded else {
                panic!("loaded source");
            };
            assert_eq!(loaded.source_id().as_str(), "include:part.adoc");
            assert_eq!(loaded.source(), "part\n");
            assert_eq!(
                loaded.provenance().canonical_path(),
                root.0.join("part.adoc")
            );
        }
        assert_eq!(session.budget().files(), 1);
    }

    #[test]
    fn failed_common_read_cannot_produce_a_loaded_source() {
        let root = TestDirectory::new();
        let mut session = session(&root.0);
        let result = IncludeFilesystem::new().read(
            &mut session,
            IncludeFilesystemRequest::new(
                LogicalSourceId::new("include:missing.adoc").expect("source ID"),
                &root.0,
                "missing.adoc",
            ),
        );
        assert!(matches!(result, IncludeFilesystemOutcome::NotFound(_)));
        assert_eq!(session.budget().files(), 0);
    }

    #[test]
    fn common_loader_failure_retains_prior_and_failed_attempt_evidence() {
        let root = TestDirectory::new();
        let loaded = root.0.join("first.adoc");
        let missing = root.0.join("missing.adoc");
        fs::write(&loaded, "first\n").expect("loaded include");
        let mut filesystem = session(&root.0);

        let failure = match prepare_with_session(
            PrepareRequest::general(
                "include::first.adoc[]\ninclude::missing.adoc[]\n",
                root.0.join("root.adoc").to_string_lossy().into_owned(),
                &root.0,
                &[],
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            &mut filesystem,
        ) {
            Ok(_) => panic!("missing required include must stop common loading"),
            Err(failure) => failure,
        };
        assert_eq!(
            failure
                .dependency_entries()
                .map(|(path, source)| (path.to_owned(), source.map(str::to_owned)))
                .collect::<Vec<_>>(),
            [
                (loaded, Some("first\n".to_owned())),
                (missing.clone(), None),
            ]
        );
        assert!(matches!(
            failure.into_error(),
            LocalIncludeError::Host(ResourceError::Missing(path)) if path == missing
        ));
    }

    #[test]
    fn missing_dependency_journal_stays_inside_root() {
        let root = TestDirectory::new();
        let mut filesystem = session(&root.0);
        let prepared = prepare_with_session(
            PrepareRequest::project(
                "include::chapters/new.adoc[]\n",
                root.0.join("root.adoc").to_string_lossy().into_owned(),
                &root.0,
                &root.0,
                &root.0,
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            &mut filesystem,
        )
        .expect("missing include is a typed validation failure");
        assert!(prepared.validation().is_some());
        let paths = prepared
            .dependency_entries()
            .map(|(path, _)| path.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(paths, [root.0.join("chapters/new.adoc")]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn existing_session_keeps_local_validation_namespace_after_root_replacement() {
        let parent = TestDirectory::new();
        let root = parent.0.join("workspace");
        fs::create_dir(&root).expect("workspace");
        fs::write(root.join("root.adoc"), "image::asset.png[]\n").expect("document");
        let mut filesystem = session(&root);
        let displaced = parent.0.join("retained-workspace");
        fs::rename(&root, &displaced).expect("displace workspace");
        fs::create_dir(&root).expect("replacement workspace");
        fs::write(root.join("asset.png"), "outside").expect("replacement target");

        let prepared = prepare_with_session(
            PrepareRequest::project(
                "image::asset.png[]\n",
                root.join("root.adoc").to_string_lossy().into_owned(),
                &root,
                &root,
                &root,
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            &mut filesystem,
        )
        .expect("prepared input");
        assert!(prepared.validation().is_some());
        let error = crate::local_target::inspect_with_session(
            &root.join("root.adoc").to_string_lossy(),
            &root,
            &root,
            "asset.png",
            &mut filesystem,
        )
        .expect_err("replacement target must remain outside the retained namespace");
        assert!(matches!(error, LocalTargetError::Missing(_)));
        fs::remove_dir_all(&root).expect("remove replacement workspace");
        fs::rename(displaced, &root).expect("restore workspace");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn existing_session_does_not_expand_an_allowed_root_after_replacement() {
        use std::os::unix::fs::symlink;

        let parent = TestDirectory::new();
        let workspace = parent.0.join("workspace");
        let allowed = workspace.join("public");
        fs::create_dir_all(&allowed).expect("allowed root");
        fs::write(workspace.join("secret.adoc"), "secret\n").expect("secret");
        let mut filesystem = LocalFilesystemPolicy::new(
            [workspace.clone(), allowed.clone()],
            FilesystemReadLimits::default(),
        )
        .and_then(|policy| policy.session())
        .expect("session");
        let retained = workspace.join("retained-public");
        fs::rename(&allowed, &retained).expect("retain original allowed root");
        symlink(&workspace, &allowed).expect("replace allowed root");

        let result = prepare_with_session(
            PrepareRequest::general(
                "include::secret.adoc[]\n",
                workspace.join("root.adoc").to_string_lossy().into_owned(),
                &workspace,
                std::slice::from_ref(&allowed),
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            &mut filesystem,
        );
        let Err(error) = result else {
            panic!("replacement must not broaden the retained allowed root");
        };
        assert!(matches!(
            error.into_error(),
            LocalIncludeError::OutsideRoot(_)
        ));
    }

    #[test]
    fn common_file_limit_keeps_the_configured_limit_in_cli_diagnostics() {
        assert_eq!(
            include_target_error(ResourceError::FileLimit { limit: 7 }),
            LocalTargetError::LimitExceeded { limit: 7 }
        );
    }

    #[test]
    fn common_driver_preserves_strategy_specific_context() {
        let root = TestDirectory::new();
        fs::write(root.0.join("part.adoc"), "part\n").expect("fixture");
        let source = "include::part.adoc[]\n";
        let source_id = root.0.join("root.adoc").to_string_lossy().into_owned();

        let regular = prepare(
            PrepareRequest::general(
                source,
                source_id.clone(),
                &root.0,
                &[],
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            FilesystemReadLimits::default(),
        )
        .expect("regular preparation");
        let mut filesystem = session(&root.0);
        let local = prepare_with_session(
            PrepareRequest::project(
                source,
                source_id,
                &root.0,
                &root.0,
                &root.0,
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            &mut filesystem,
        )
        .expect("local preparation");

        assert_eq!(
            regular.projection().document().source,
            local.projection().document().source
        );
        assert!(regular.validation.is_none());
        assert!(local.validation.is_some());
        let observed = local.dependency_entries().collect::<Vec<_>>();
        assert!(observed.iter().any(|(path, source)| {
            *path == root.0.join("part.adoc") && *source == Some("part\n")
        }));
    }

    #[test]
    fn common_driver_uses_the_project_analysis_attributes() {
        let root = TestDirectory::new();
        fs::write(root.0.join("part.adoc"), "part\n").expect("fixture");
        let attributes = BTreeMap::from([("selected".to_owned(), Some("part".to_owned()))]);
        let mut analysis = adocweave::AnalysisOptions::default();
        analysis.attributes.clone_from(&attributes);
        let preprocess = PreprocessOptions {
            attributes,
            ..PreprocessOptions::default()
        };

        let prepared = prepare(
            PrepareRequest::general(
                "include::{selected}.adoc[]\n",
                "root.adoc".to_owned(),
                &root.0,
                &[],
                &preprocess,
                &analysis,
            ),
            FilesystemReadLimits::default(),
        )
        .expect("matching project settings");

        assert_eq!(prepared.projection().document().source, "part\n");
    }

    #[test]
    fn include_reads_and_local_inspection_share_one_path_limit() {
        let root = TestDirectory::new();
        fs::write(root.0.join("part.adoc"), "part\n").expect("include fixture");
        fs::write(root.0.join("asset.png"), "asset").expect("target fixture");
        let limits = FilesystemReadLimits {
            max_files: 1,
            ..FilesystemReadLimits::default()
        };
        let mut filesystem = LocalFilesystemPolicy::new([root.0.clone()], limits)
            .and_then(|policy| policy.session())
            .expect("session");
        let prepared = prepare_with_session(
            PrepareRequest::project(
                "include::part.adoc[]\nimage::asset.png[]\n",
                "root.adoc".to_owned(),
                &root.0,
                &root.0,
                &root.0,
                &PreprocessOptions::default(),
                &adocweave::AnalysisOptions::default(),
            ),
            &mut filesystem,
        )
        .expect("include preparation");
        assert!(prepared.validation().is_some());

        assert_eq!(
            crate::local_target::inspect_with_session(
                "root.adoc",
                &root.0,
                &root.0,
                "asset.png",
                &mut filesystem,
            ),
            Err(LocalTargetError::LimitExceeded { limit: 1 })
        );
    }

    #[cfg(unix)]
    #[test]
    fn aliases_keep_distinct_logical_ids_and_share_canonical_provenance() {
        use std::os::unix::fs::symlink;

        let root = TestDirectory::new();
        fs::write(root.0.join("part.adoc"), "part\n").expect("fixture");
        symlink("part.adoc", root.0.join("alias.adoc")).expect("alias");
        let mut session = session(&root.0);

        let read = |session: &mut LocalFilesystemSession, id: &str, target: &str| {
            let IncludeFilesystemOutcome::Found(found) = IncludeFilesystem::new().read(
                session,
                IncludeFilesystemRequest::new(
                    LogicalSourceId::new(id).expect("source ID"),
                    &root.0,
                    target,
                ),
            ) else {
                panic!("loaded source");
            };
            found
        };
        let direct = read(&mut session, "include:part.adoc", "part.adoc");
        let alias = read(&mut session, "include:alias.adoc", "alias.adoc");

        assert_ne!(direct.source_id(), alias.source_id());
        assert_eq!(
            direct.provenance().canonical_path(),
            alias.provenance().canonical_path()
        );
        assert_eq!(session.budget().files(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_fails_before_loaded_source_construction() {
        use std::os::unix::fs::symlink;

        let root = TestDirectory::new();
        let outside = TestDirectory::new();
        fs::write(outside.0.join("outside.adoc"), "outside\n").expect("outside fixture");
        symlink(outside.0.join("outside.adoc"), root.0.join("escape.adoc")).expect("escape");
        let mut session = session(&root.0);
        let outcome = IncludeFilesystem::new().read(
            &mut session,
            IncludeFilesystemRequest::new(
                LogicalSourceId::new("include:escape.adoc").expect("source ID"),
                &root.0,
                "escape.adoc",
            ),
        );
        assert!(matches!(outcome, IncludeFilesystemOutcome::Failed(_)));
        assert_eq!(session.budget().files(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn logical_ancestor_symlink_keeps_a_verified_missing_watch_candidate() {
        use std::os::unix::fs::symlink;

        let root = TestDirectory::new();
        fs::create_dir(root.0.join("dir-a")).expect("dir a");
        symlink("dir-a", root.0.join("current")).expect("logical symlink");
        let mut session = session(&root.0);
        let outcome = IncludeFilesystem::new().read(
            &mut session,
            IncludeFilesystemRequest::new(
                LogicalSourceId::new("include:current/part.adoc").expect("source ID"),
                &root.0,
                "current/part.adoc",
            ),
        );
        let IncludeFilesystemOutcome::NotFound(missing) = outcome else {
            panic!("missing target");
        };
        assert_eq!(
            missing.watch_candidate().path(),
            root.0.join("current/part.adoc")
        );
    }
}
