//! Explicit, bounded local resource provider owned by the CLI binary.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use adocweave::preprocess::{
    EffectiveProcessingOptions, PreprocessError, PreprocessOptions, PreprocessedDocument,
};
use adocweave_host::{
    FilesystemReadLimits, IncludeFilesystem, IncludeFilesystemOutcome, IncludeFilesystemRequest,
    LocalFilesystemPolicy, LocalFilesystemSession, LocalTargetError, LocalTargetPolicy,
    LogicalSourceId, ResourceError,
};
use adocweave_workspace::{
    NeverCancelled, ResourceId, Revision, Workspace, WorkspaceIncludeResolution, WorkspaceLimits,
    WorkspacePreprocessDraft, WorkspacePreprocessStep,
};

#[derive(Debug)]
pub enum LocalIncludeError {
    InvalidBase {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidRoot {
        path: PathBuf,
        source: std::io::Error,
    },
    OutsideRoot(PathBuf),
    Position(adocweave::text::PositionError),
    Preprocess(PreprocessError),
    Analysis(String),
    MissingSource(String),
    Host(ResourceError),
}

pub struct PreparedInput {
    projection: ProjectionInput,
    validation: Option<LocalValidationContext>,
}

pub struct ProjectionInput {
    draft: WorkspacePreprocessDraft,
    source_keys: BTreeMap<String, ResourceId>,
    source_bases: BTreeMap<String, PathBuf>,
    include_bases: BTreeMap<String, PathBuf>,
}

pub struct LocalValidationContext {
    authority: PathBuf,
    include_errors: Vec<IncludeFailure>,
}

#[derive(Clone, Debug)]
struct IncludeFailure {
    source_id: Option<String>,
    range: adocweave::text::TextRange,
    target: String,
    error: LocalTargetError,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DependencyJournal {
    entries: BTreeMap<PathBuf, Option<Arc<str>>>,
}

impl DependencyJournal {
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
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

    pub fn projection_and_validation_mut(
        &mut self,
    ) -> (&ProjectionInput, Option<&mut LocalValidationContext>) {
        (&self.projection, self.validation.as_mut())
    }

    pub(crate) fn resource_sizes(&self) -> impl Iterator<Item = u64> + '_ {
        self.projection.resource_lengths()
    }

    pub(crate) fn resource_entries(&self) -> impl Iterator<Item = (&str, u64)> + '_ {
        self.projection.source_keys.iter().filter_map(|(id, key)| {
            self.projection
                .draft
                .source(key)
                .map(|source| (id.as_str(), source.len() as u64))
        })
    }
}

impl ProjectionInput {
    pub fn document(&self) -> &PreprocessedDocument {
        self.draft.document()
    }

    pub fn source(&self, source_id: &str) -> Option<&str> {
        self.source_keys
            .get(source_id)
            .and_then(|key| self.draft.source(key))
    }

    pub fn source_base(&self, source_id: &str) -> Option<&Path> {
        self.source_bases.get(source_id).map(PathBuf::as_path)
    }

    pub fn include_base(&self, source_id: &str) -> Option<&Path> {
        self.include_bases.get(source_id).map(PathBuf::as_path)
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
    pub fn authority(&self) -> &Path {
        &self.authority
    }

    pub fn include_error(
        &self,
        source_id: &str,
        range: adocweave::text::TextRange,
        target: &str,
    ) -> Option<&LocalTargetError> {
        self.include_errors
            .iter()
            .find(|failure| {
                failure.source_id.as_deref() == Some(source_id)
                    && failure.range == range
                    && failure.target == target
            })
            .map(|failure| &failure.error)
    }

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

impl fmt::Display for LocalIncludeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase { path, source } => {
                write!(
                    formatter,
                    "invalid include base {}: {source}",
                    path.display()
                )
            }
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
            Self::Position(error) => error.fmt(formatter),
            Self::Preprocess(error) => error.fmt(formatter),
            Self::Analysis(error) => formatter.write_str(error),
            Self::MissingSource(source_id) => {
                write!(formatter, "projected source is missing: {source_id}")
            }
            Self::Host(error) => error.fmt(formatter),
        }
    }
}

impl Error for LocalIncludeError {}

enum IncludeReadMode {
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
    fn local_authority(&self) -> Option<&Path> {
        match self {
            Self::Local { root } => Some(root),
            Self::General { .. } => None,
        }
    }

    fn watch_candidate(&self, target: &str) -> Option<PathBuf> {
        match self {
            Self::Local { root } => Some(root.join(target)),
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

#[allow(clippy::too_many_arguments)]
fn prepare_with_driver(
    source: &str,
    source_id: String,
    source_base: PathBuf,
    include_base: Option<PathBuf>,
    mut preprocess_options: PreprocessOptions,
    analysis_options: &adocweave::AnalysisOptions,
    read_mode: IncludeReadMode,
    filesystem: &mut LocalFilesystemSession,
    validate_local_targets: bool,
    dependencies: &mut DependencyJournal,
) -> Result<PreparedInput, LocalIncludeError> {
    let validation_authority = read_mode.local_authority().map(Path::to_owned);
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
    let mut source_keys = BTreeMap::from([(source_id.clone(), root_id.clone())]);
    let mut source_bases = BTreeMap::from([(source_id.clone(), source_base)]);
    let mut include_bases = include_base
        .map(|base| BTreeMap::from([(source_id.clone(), base)]))
        .unwrap_or_default();
    let mut failure_errors = BTreeMap::new();
    let mut step = workspace
        .snapshot()
        .preprocess_resumable(&root_id, &options, &NeverCancelled);
    let draft = loop {
        match step {
            WorkspacePreprocessStep::Complete(draft) => break *draft,
            WorkspacePreprocessStep::NeedResource(suspended) => {
                let request = suspended.request();
                let target = request.target().to_owned();
                let projected_source_id = include_source_id(&target);
                let logical_id = LogicalSourceId::new(projected_source_id.clone())
                    .map_err(LocalIncludeError::Host)?;
                let request_range = request.range();
                let inspect = !validate_local_targets || {
                    adocweave::LocalTargetReference::from_include(
                        request_range,
                        request_range,
                        request.authored_target(),
                    )
                    .is_some_and(|reference| {
                        reference.syntax == adocweave::LocalTargetSyntax::Candidate
                    })
                };
                let outcome = if inspect {
                    if let Some(candidate) = read_mode.watch_candidate(&target) {
                        dependencies.observe_candidate(&candidate);
                    }
                    read_mode.read(filesystem, logical_id, &target)?
                } else {
                    let error = LocalTargetError::Unverifiable(target.clone());
                    failure_errors.insert(target.clone(), error);
                    let response = request.failed_with_placeholder_as(projected_source_id);
                    step = suspended.resume(response, &NeverCancelled);
                    continue;
                };
                let response = match outcome {
                    IncludeFilesystemOutcome::Found(found) => {
                        let source = Arc::<str>::from(found.source());
                        for candidate in found.watch_candidates() {
                            dependencies.observe_candidate(candidate.path());
                        }
                        dependencies.observe_loaded(
                            found.provenance().canonical_path(),
                            Arc::clone(&source),
                        );
                        let key = ResourceId::new(target.clone())
                            .map_err(|error| LocalIncludeError::Analysis(error.to_string()))?;
                        source_keys.insert(projected_source_id.clone(), key);
                        let base = found
                            .provenance()
                            .canonical_path()
                            .parent()
                            .unwrap_or_else(|| Path::new(""))
                            .to_owned();
                        source_bases.insert(projected_source_id.clone(), base.clone());
                        if validate_local_targets {
                            include_bases.insert(projected_source_id.clone(), base);
                        }
                        request.found_as(projected_source_id, source)
                    }
                    IncludeFilesystemOutcome::NotFound(missing) => {
                        dependencies.observe_candidate(missing.watch_candidate().path());
                        let error =
                            LocalTargetError::Missing(missing.watch_candidate().path().to_owned());
                        if !validate_local_targets {
                            return Err(LocalIncludeError::Host(ResourceError::Missing(
                                missing.watch_candidate().path().to_owned(),
                            )));
                        }
                        failure_errors.insert(target, error);
                        request.failed_with_placeholder_as(projected_source_id)
                    }
                    IncludeFilesystemOutcome::Failed(failed) => {
                        let host_error = ResourceError::from(failed.error().clone());
                        if !validate_local_targets {
                            return Err(LocalIncludeError::Host(host_error));
                        }
                        failure_errors.insert(target, include_target_error(host_error));
                        request.failed_with_placeholder_as(projected_source_id)
                    }
                };
                step = suspended.resume(response, &NeverCancelled);
            }
            WorkspacePreprocessStep::Failed(failure) => {
                return Err(LocalIncludeError::Analysis(failure.error().to_string()));
            }
            WorkspacePreprocessStep::Cancelled => {
                return Err(LocalIncludeError::Analysis(
                    "include preprocessing was cancelled".to_owned(),
                ));
            }
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
                    source_id: event.source_id().map(str::to_owned),
                    range: event.range(),
                    target: event.target().to_string(),
                    error,
                })
        })
        .collect();
    let validation = validate_local_targets.then(|| LocalValidationContext {
        authority: validation_authority.expect("local validation has an authority"),
        include_errors,
    });
    Ok(PreparedInput {
        projection: ProjectionInput {
            draft,
            source_keys,
            source_bases,
            include_bases,
        },
        validation,
    })
}

pub fn prepare(
    source: &str,
    source_id: Option<String>,
    base_dir: &Path,
    allowed_roots: &[PathBuf],
    limits: FilesystemReadLimits,
    preprocess_options: &PreprocessOptions,
    analysis_options: &adocweave::AnalysisOptions,
) -> Result<PreparedInput, LocalIncludeError> {
    let base_dir = base_dir
        .canonicalize()
        .map_err(|source| LocalIncludeError::InvalidBase {
            path: base_dir.to_owned(),
            source,
        })?;
    let allowed_roots = if allowed_roots.is_empty() {
        Vec::new()
    } else {
        allowed_roots
            .iter()
            .map(|path| {
                path.canonicalize()
                    .map_err(|source| LocalIncludeError::InvalidRoot {
                        path: path.clone(),
                        source,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut session_roots = allowed_roots.clone();
    if !session_roots.contains(&base_dir) {
        session_roots.push(base_dir.clone());
    }
    let policy =
        LocalFilesystemPolicy::new(session_roots, limits).map_err(LocalIncludeError::Host)?;
    let mut filesystem = policy.session().map_err(LocalIncludeError::Host)?;
    prepare_with_session(
        source,
        source_id,
        &base_dir,
        &allowed_roots,
        preprocess_options,
        analysis_options,
        &mut filesystem,
    )
}

pub(crate) fn prepare_with_session(
    source: &str,
    source_id: Option<String>,
    base_dir: &Path,
    allowed_roots: &[PathBuf],
    preprocess_options: &PreprocessOptions,
    analysis_options: &adocweave::AnalysisOptions,
    filesystem: &mut LocalFilesystemSession,
) -> Result<PreparedInput, LocalIncludeError> {
    let base_policy = filesystem
        .policy_for_path(base_dir)
        .ok_or_else(|| LocalIncludeError::OutsideRoot(base_dir.to_owned()))?
        .derive_confined_directory(base_dir)
        .map_err(|error| LocalIncludeError::Analysis(format!("invalid include base: {error}")))?;
    let base_dir = base_policy.root().to_owned();
    let allowed_policies = allowed_roots
        .iter()
        .map(|path| {
            filesystem
                .policy_for_path(path)
                .ok_or_else(|| LocalIncludeError::OutsideRoot(path.clone()))?
                .derive_confined_directory(path)
                .map_err(|error| {
                    LocalIncludeError::Analysis(format!("invalid include root: {error}"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    prepare_with_driver(
        source,
        source_id.unwrap_or_else(|| "<stdin>".to_owned()),
        base_dir.clone(),
        None,
        preprocess_options.clone(),
        analysis_options,
        IncludeReadMode::General {
            base: base_dir,
            base_policy,
            allowed: allowed_policies,
        },
        filesystem,
        false,
        &mut DependencyJournal::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_local(
    source: &str,
    source_id: String,
    base_dir: &Path,
    source_base: &Path,
    project_root: &Path,
    limits: FilesystemReadLimits,
    preprocess_options: &PreprocessOptions,
    analysis_options: &adocweave::AnalysisOptions,
) -> Result<PreparedInput, LocalIncludeError> {
    let filesystem_policy = LocalFilesystemPolicy::new([project_root.to_owned()], limits)
        .map_err(LocalIncludeError::Host)?;
    let mut filesystem = filesystem_policy
        .session()
        .map_err(LocalIncludeError::Host)?;
    prepare_local_with_session(
        source,
        source_id,
        base_dir,
        source_base,
        project_root,
        preprocess_options,
        analysis_options,
        &mut filesystem,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_local_with_session(
    source: &str,
    source_id: String,
    base_dir: &Path,
    source_base: &Path,
    project_root: &Path,
    preprocess_options: &PreprocessOptions,
    analysis_options: &adocweave::AnalysisOptions,
    filesystem_session: &mut LocalFilesystemSession,
) -> Result<PreparedInput, LocalIncludeError> {
    prepare_local_tracking_with_existing_session(
        source,
        source_id,
        base_dir,
        source_base,
        project_root,
        preprocess_options,
        analysis_options,
        &mut DependencyJournal::default(),
        filesystem_session,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_local_tracking_with_existing_session(
    source: &str,
    source_id: String,
    base_dir: &Path,
    source_base: &Path,
    project_root: &Path,
    preprocess_options: &PreprocessOptions,
    analysis_options: &adocweave::AnalysisOptions,
    dependencies: &mut DependencyJournal,
    filesystem_session: &mut LocalFilesystemSession,
) -> Result<PreparedInput, LocalIncludeError> {
    let policy = filesystem_session
        .policy_for_path(project_root)
        .ok_or_else(|| LocalIncludeError::OutsideRoot(project_root.to_owned()))?
        .derive_confined_directory(project_root)
        .map_err(|error| LocalIncludeError::Analysis(format!("invalid project root: {error}")))?;
    let base_dir = policy
        .inspect_directory_no_symlinks(base_dir)
        .map_err(|error| LocalIncludeError::Analysis(format!("invalid include base: {error}")))?;
    let root = policy.root().to_owned();
    let base_key = logical_key(
        base_dir
            .strip_prefix(&root)
            .expect("base checked below root"),
    );

    let source_base = policy
        .inspect_directory_no_symlinks(source_base)
        .map_err(|error| LocalIncludeError::Analysis(format!("invalid source base: {error}")))?;
    let mut preprocess_options = preprocess_options.clone();
    preprocess_options.base_uri = (!base_key.is_empty()).then_some(base_key);
    prepare_with_driver(
        source,
        source_id,
        source_base,
        Some(base_dir),
        preprocess_options,
        analysis_options,
        IncludeReadMode::Local { root },
        filesystem_session,
        true,
        dependencies,
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
    fn missing_dependency_journal_stays_inside_root() {
        let root = TestDirectory::new();
        let mut filesystem = session(&root.0);
        let mut dependencies = DependencyJournal::default();
        let prepared = prepare_local_tracking_with_existing_session(
            "include::chapters/new.adoc[]\n",
            root.0.join("root.adoc").to_string_lossy().into_owned(),
            &root.0,
            &root.0,
            &root.0,
            &PreprocessOptions::default(),
            &adocweave::AnalysisOptions::default(),
            &mut dependencies,
            &mut filesystem,
        )
        .expect("missing include is a typed validation failure");
        assert!(prepared.validation().is_some());
        let paths = dependencies
            .entries()
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

        let prepared = prepare_local_with_session(
            "image::asset.png[]\n",
            root.join("root.adoc").to_string_lossy().into_owned(),
            &root,
            &root,
            &root,
            &PreprocessOptions::default(),
            &adocweave::AnalysisOptions::default(),
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
            "include::secret.adoc[]\n",
            Some(workspace.join("root.adoc").to_string_lossy().into_owned()),
            &workspace,
            std::slice::from_ref(&allowed),
            &PreprocessOptions::default(),
            &adocweave::AnalysisOptions::default(),
            &mut filesystem,
        );
        let Err(error) = result else {
            panic!("replacement must not broaden the retained allowed root");
        };
        assert!(matches!(error, LocalIncludeError::OutsideRoot(_)));
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
            source,
            Some(source_id.clone()),
            &root.0,
            &[],
            FilesystemReadLimits::default(),
            &PreprocessOptions::default(),
            &adocweave::AnalysisOptions::default(),
        )
        .expect("regular preparation");
        let mut filesystem = session(&root.0);
        let mut dependencies = DependencyJournal::default();
        let local = prepare_local_tracking_with_existing_session(
            source,
            source_id,
            &root.0,
            &root.0,
            &root.0,
            &PreprocessOptions::default(),
            &adocweave::AnalysisOptions::default(),
            &mut dependencies,
            &mut filesystem,
        )
        .expect("local preparation");

        assert_eq!(
            regular.projection().document().source,
            local.projection().document().source
        );
        assert!(regular.validation.is_none());
        assert!(local.validation.is_some());
        let observed = dependencies.entries().collect::<Vec<_>>();
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
            "include::{selected}.adoc[]\n",
            Some("root.adoc".to_owned()),
            &root.0,
            &[],
            FilesystemReadLimits::default(),
            &preprocess,
            &analysis,
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
        let prepared = prepare_local_with_session(
            "include::part.adoc[]\nimage::asset.png[]\n",
            "root.adoc".to_owned(),
            &root.0,
            &root.0,
            &root.0,
            &PreprocessOptions::default(),
            &adocweave::AnalysisOptions::default(),
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
