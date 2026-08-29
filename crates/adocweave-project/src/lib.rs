//! Owned contracts for processing one project request.
//!
//! A [`ProjectRequest`] contains every selected target, caller-provided source,
//! configuration choice, verified [`ProjectAuthority`] and finite limits for
//! one run. The result owns analyses, dependency observations and safe watch
//! candidates. LSP revisions remain in the caller's job state and are not
//! echoed through this crate.
//!
//! A [`ProjectOutcome`] separates failures which invalidate the whole request
//! from [`ProjectTargetError`] values for one selected document. Successful
//! results may still contain [`ProjectWarning`] values when a bounded scan
//! returns the targets it found before reaching its limit.
//!
//! This crate defines no long-lived service or shared state. [`process`]
//! consumes one request, fixes each observed file result for that call and
//! returns all owned results before it finishes.

mod process;
mod selection;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adocweave::output::formatter::FormatConfig;
use adocweave::output::html::RenderPolicy;
use adocweave::preprocess::PreprocessOptions;
use adocweave::preprocess::{AnalysisProjection, PreprocessedAnalysis, PreprocessedAnalysisError};
use adocweave::{Analysis, AnalysisOptions, SourceId};
use adocweave_config::{ConfigError, ConfigErrorCode, ConfigSnapshot, ResolvedProjectConfig};
use adocweave_host::{
    DerivedFilesystemRoots, FilesystemReadLimits, LocalFilesystemPolicy, ResourceError,
};

pub use process::{process, resolve_config};

pub(crate) fn project_authority_error(error: ResourceError) -> ProjectError {
    ProjectError::Authority(ProjectResourceError::from_host(error))
}

pub(crate) fn project_config_error(error: ConfigError) -> ProjectError {
    ProjectError::Config(ProjectConfigError::from_config(error))
}

pub(crate) fn project_target_read(error: ResourceError) -> ProjectTargetError {
    ProjectTargetError::Read(ProjectResourceError::from_host(error))
}

/// Fixed request ceiling applied before compiling or scanning glob selectors.
///
/// This is not part of [`ProjectLimits`]: accepting a request must itself stay
/// bounded before request-controlled filesystem and processing limits can be
/// applied.
pub(crate) const MAX_DISTINCT_GLOB_SELECTORS: usize = 256;

/// Fixed ceiling for the UTF-8 bytes in distinct authored glob patterns.
///
/// Duplicate patterns are counted once. Exceeding this ceiling is a target
/// selection error known before any glob is compiled or any directory is read.
pub(crate) const MAX_TOTAL_GLOB_PATTERN_BYTES: usize = 64 * 1024;

/// One owned request covering every target selected for a single run.
///
/// `authority` retains the opened project root and the maximum filesystem
/// scope. Paths read from configuration may narrow this scope but cannot widen
/// it. [`ProjectSource`] values replace primary/include bodies only; they never
/// replace configuration, stylesheets or local-target inspection.
#[derive(Debug)]
pub struct ProjectRequest {
    pub targets: Vec<ProjectTarget>,
    /// In-memory sources available as primary documents or include overlays.
    pub sources: Vec<ProjectSource>,
    pub config: ConfigSelection,
    pub overrides: ProjectOverrides,
    pub authority: ProjectAuthority,
    pub limits: ProjectLimits,
}

/// Verified filesystem authority for one project request.
///
/// Constructing this value opens the permitted roots. A project configuration
/// may narrow those roots, but cannot add authority later.
#[derive(Clone, Debug)]
pub struct ProjectAuthority {
    project_root: PathBuf,
    policy: LocalFilesystemPolicy,
}

impl ProjectAuthority {
    /// Opens `roots` and verifies that `project_root` is inside one of them.
    pub fn open(
        project_root: impl Into<PathBuf>,
        roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, ProjectResourceError> {
        let project_root = project_root.into();
        if !project_root.is_absolute() {
            return Err(ProjectResourceError::from_host(
                ResourceError::PathNotAbsolute(project_root),
            ));
        }
        let mut policy = LocalFilesystemPolicy::new(roots, FilesystemReadLimits::default())
            .map_err(ProjectResourceError::from_host)?;
        let project_root = project_root.canonicalize().map_err(|error| {
            ProjectResourceError::from_host(ResourceError::Inspect {
                path: project_root.clone(),
                source: error.to_string(),
            })
        })?;
        let anchor = policy
            .policy_for_path(&project_root)
            .map(|value| value.root().to_owned());
        let Some(anchor) = anchor else {
            return Err(ProjectResourceError::from_host(
                ResourceError::OutsideRoots(project_root),
            ));
        };
        if anchor != project_root {
            policy
                .access_derived(
                    &anchor,
                    DerivedFilesystemRoots {
                        confined: vec![project_root.clone()],
                        independent: Vec::new(),
                    },
                    FilesystemReadLimits::default(),
                )
                .map_err(ProjectResourceError::from_host)?;
        }
        Ok(Self {
            project_root,
            policy,
        })
    }

    /// Returns the trusted project boundary.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) fn into_parts(self) -> (PathBuf, LocalFilesystemPolicy) {
        (self.project_root, self.policy)
    }
}

/// A target selector independent of command-line argument types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectTarget {
    /// One source supplied in [`ProjectRequest::sources`].
    Source(SourceId),
    /// One authored file path.
    Path(PathBuf),
    /// Every supported document selected below one directory.
    Directory(PathBuf),
    /// Files selected by one authored glob pattern.
    Glob(String),
    /// Supported documents found below the project root by workspace discovery
    /// with configured excludes.
    Workspace(PathBuf),
}

/// One fixed in-memory source and its include-resolution path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSource {
    pub source_id: SourceId,
    /// File replaced by this source, or `None` for standard input.
    pub path: Option<PathBuf>,
    /// Directory used to resolve relative includes.
    pub base: PathBuf,
    pub source: Arc<str>,
}

impl ProjectSource {
    pub fn new(source_id: SourceId, path: PathBuf, source: impl Into<Arc<str>>) -> Self {
        let base = path.parent().map(Path::to_owned).unwrap_or_default();
        Self {
            source_id,
            path: Some(path),
            base,
            source: source.into(),
        }
    }

    /// Creates a pathless source such as standard input.
    pub fn memory(source_id: SourceId, base: PathBuf, source: impl Into<Arc<str>>) -> Self {
        Self {
            source_id,
            path: None,
            base,
            source: source.into(),
        }
    }
}

/// How a request selects its project configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ConfigSelection {
    /// Searches from each target towards [`ProjectAuthority::project_root`].
    #[default]
    Discover,
    /// Uses one explicitly selected project file.
    Explicit(PathBuf),
    /// Uses built-in defaults without loading a project file.
    Disabled,
}

/// Request-local changes applied after resolving project configuration.
///
/// `None` preserves the configured value. This type contains only settings
/// which currently have a caller-level override; it is not a second project
/// configuration schema.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectOverrides {
    pub include: Option<bool>,
    /// Additional stylesheet files resolved from the project root and observed
    /// under the same budgets as configured stylesheets.
    pub stylesheet_files: Vec<PathBuf>,
}

/// Hard ceilings shared by every target and physical acquisition in one request.
///
/// Include depth, include count and parser limits remain part of the resolved
/// project configuration. Configured resource limits are narrower budgets
/// shared by targets which resolve the same configuration snapshot; they do
/// not replace these request-wide ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectLimits {
    pub max_files: usize,
    pub max_resource_bytes: u64,
    pub max_read_bytes: u64,
    pub max_directory_entries: u64,
    pub max_processing_iterations: u32,
    /// Maximum UTF-8 bytes retained in returned expanded documents and loaded
    /// resource bodies. Rendered products added by later stages have their own
    /// output accounting and are not charged here.
    pub max_output_bytes: u32,
}

impl ProjectLimits {
    pub(crate) const fn filesystem_reads(self) -> FilesystemReadLimits {
        FilesystemReadLimits {
            max_files: self.max_files,
            max_total_bytes: self.max_read_bytes,
            max_resource_bytes: self.max_resource_bytes,
        }
    }
}

/// Result of attempting one complete project request.
///
/// An error means the request could not establish a safe common basis, such as
/// valid configuration or filesystem authority. Errors isolated to one target
/// are stored in [`ProjectTargetResult::outcome`] instead.
pub type ProjectOutcome = Result<ProjectResult, ProjectError>;

/// Owned results and accounting for one completed request.
#[derive(Debug)]
pub struct ProjectResult {
    /// Results in the stable target order established by project processing.
    pub targets: Vec<ProjectTargetResult>,
    /// Request-wide configuration discovery observations, including missing
    /// candidates that are safe to watch.
    pub resources: Vec<ProjectResourceResult>,
    /// Recoverable request-wide conditions which leave partial results usable.
    pub warnings: Vec<ProjectWarning>,
    /// Resource use shared by all targets in this request.
    pub usage: ProjectUsage,
}

/// Request which resolves configuration without selecting or analyzing a document.
#[derive(Debug)]
pub struct ProjectConfigRequest {
    pub authority: ProjectAuthority,
    pub search_from: PathBuf,
    pub search_from_is_directory: bool,
    pub config: ConfigSelection,
    pub overrides: ProjectOverrides,
    pub limits: ProjectLimits,
}

/// Configuration-only result with the observations needed for invalidation.
#[derive(Debug)]
pub struct ProjectConfigResult {
    pub config: Arc<ProjectConfigSnapshot>,
    pub resources: Vec<ProjectResourceResult>,
    pub warnings: Vec<ProjectWarning>,
    pub usage: ProjectUsage,
}

/// Result for one concrete document selected from the request.
#[derive(Debug)]
pub struct ProjectTargetResult {
    pub source_id: SourceId,
    /// Filesystem path for file-backed input, or `None` for a source supplied
    /// directly by the caller.
    pub path: Option<PathBuf>,
    /// Original main-document text when it fits the result limit.
    pub source: Option<Arc<str>>,
    /// Effective configuration and its optional source identity.
    pub config: Arc<ProjectConfigSnapshot>,
    /// Files read or inspected on behalf of this target.
    pub resources: Vec<ProjectResourceResult>,
    pub outcome: Result<ProjectAnalysis, ProjectTargetError>,
}

/// Analyses derived once from one fixed set of project inputs.
#[derive(Debug)]
pub struct ProjectAnalysis {
    /// Analysis of the unexpanded primary source.
    pub source: Analysis,
    /// Analysis of the include-expanded document and its source map.
    pub preprocessed: PreprocessedAnalysis,
    /// Editor-facing facts mapped back to positions in the original sources.
    pub source_mapping: AnalysisProjection,
}

/// Why one filesystem resource was acquired.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectResourceKind {
    Config,
    Primary,
    Include,
    Stylesheet,
    LocalTarget,
}

/// One fixed filesystem observation made during a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectResourceResult {
    pub source_id: SourceId,
    pub path: PathBuf,
    /// Authored or discovered path before filesystem resolution.
    pub requested_path: PathBuf,
    /// Path that is safe for a caller to watch for a later change.
    ///
    /// Missing and unreadable filesystem resources remain watchable so a
    /// caller can observe their repair. Caller input, policy rejection and
    /// limit failures do not expose a watch path.
    pub watch_path: Option<PathBuf>,
    pub kind: ProjectResourceKind,
    pub origin: ProjectResourceOrigin,
    /// Source which requested this resource, including each include edge.
    pub requested_by: Option<SourceId>,
    pub outcome: ProjectResourceOutcome,
}

/// Origin of a fixed resource value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectResourceOrigin {
    Filesystem,
    Input,
}

/// Content or failure retained for one logical resource until the request ends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectResourceOutcome {
    Loaded {
        source: Arc<str>,
    },
    /// Acquisition succeeded, but returning the body would exceed the result limit.
    LoadedOmitted {
        limit: ProjectLimit,
    },
    Present,
    Missing,
    Failed(ProjectResourceFailure),
}

/// A resource failure which callers can classify without parsing text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectResourceFailure {
    Unreadable(ProjectResourceError),
    Rejected(ProjectResourceError),
    Limit(ProjectLimit),
}

impl fmt::Display for ProjectResourceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(error) | Self::Rejected(error) => error.fmt(formatter),
            Self::Limit(limit) => limit.fmt(formatter),
        }
    }
}

/// Resource use accumulated over one request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectUsage {
    pub read_operations: u64,
    pub read_bytes: u64,
    pub directory_operations: u64,
    pub directory_entries: u64,
    pub processing_iterations: u32,
    /// Expanded document and loaded resource bytes retained in this result.
    pub output_bytes: u64,
}

/// A safety ceiling reached while processing a request or target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectLimit {
    Files {
        limit: usize,
    },
    /// Maximum bytes accepted from one resource body.
    ResourceBytes {
        limit: u64,
    },
    /// Aggregate bytes read across a request or configuration scope.
    ReadBytes {
        limit: u64,
    },
    DirectoryEntries {
        limit: u64,
    },
    ProcessingIterations {
        limit: u32,
    },
    OutputBytes {
        limit: u32,
    },
}

impl fmt::Display for ProjectLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Files { limit } => write!(formatter, "project file limit exceeded: {limit}"),
            Self::ResourceBytes { limit } => {
                write!(formatter, "project resource byte limit exceeded: {limit}")
            }
            Self::ReadBytes { limit } => {
                write!(formatter, "project read byte limit exceeded: {limit}")
            }
            Self::DirectoryEntries { limit } => {
                write!(formatter, "project directory entry limit exceeded: {limit}")
            }
            Self::ProcessingIterations { limit } => {
                write!(
                    formatter,
                    "project processing iteration limit exceeded: {limit}"
                )
            }
            Self::OutputBytes { limit } => {
                write!(formatter, "project output byte limit exceeded: {limit}")
            }
        }
    }
}

impl std::error::Error for ProjectLimit {}

/// A recoverable condition which leaves safely collected target results usable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectWarning {
    /// Directory scanning stopped at its shared entry limit.
    ScanTruncated { limit: u64 },
    /// A related resource could not be represented or acquired safely.
    Resource {
        path: PathBuf,
        kind: ProjectResourceKind,
        failure: ProjectResourceFailure,
    },
    /// A local reference could not be mapped to a verifiable source position.
    LocalTargetMapping { message: String },
}

/// A target selection failure known before reading a selected document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetSelectionError {
    InvalidGlob {
        pattern: String,
    },
    /// Too many distinct patterns were supplied for one bounded request.
    TooManyGlobs {
        limit: usize,
    },
    /// Distinct authored patterns exceeded their fixed aggregate size.
    GlobPatternBytes {
        limit: usize,
    },
}

impl fmt::Display for TargetSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGlob { pattern } => write!(formatter, "invalid target glob: {pattern}"),
            Self::TooManyGlobs { limit } => {
                write!(formatter, "distinct glob selector limit exceeded: {limit}")
            }
            Self::GlobPatternBytes { limit } => {
                write!(formatter, "glob pattern byte limit exceeded: {limit}")
            }
        }
    }
}

impl std::error::Error for TargetSelectionError {}

/// Failure which prevents a coherent result for the entire request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectError {
    Config(ProjectConfigError),
    TargetSelection(TargetSelectionError),
    Authority(ProjectResourceError),
    /// The caller cancelled the request before it completed.
    Cancelled,
    InvalidInput(ProjectInputError),
    Limit(ProjectLimit),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::TargetSelection(error) => error.fmt(formatter),
            Self::Authority(error) => error.fmt(formatter),
            Self::Limit(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("project processing cancelled"),
            Self::InvalidInput(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::TargetSelection(error) => Some(error),
            Self::Authority(error) => Some(error),
            Self::Limit(error) => Some(error),
            Self::Cancelled => None,
            Self::InvalidInput(error) => Some(error),
        }
    }
}

/// Caller-owned request values are internally inconsistent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInputError {
    pub code: &'static str,
    message: String,
}

impl ProjectInputError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ProjectInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectInputError {}

/// Failure confined to one selected document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectTargetError {
    Read(ProjectResourceError),
    Analysis(PreprocessedAnalysisError),
    /// The result is incomplete and must not be presented as fully analyzed.
    Incomplete(ProjectLimit),
}

/// Stable category for a project-owned configuration error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectConfigErrorCode {
    ReadFailed,
    OutsideBoundary,
    InvalidToml,
    UnsupportedSchema,
    InvalidRule,
    InvalidAttribute,
    InvalidLimit,
    InvalidPath,
    InvalidRole,
}

/// Configuration failure without an `adocweave-config` type in the contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigError {
    pub code: ProjectConfigErrorCode,
    pub field: Option<String>,
    message: String,
}

impl ProjectConfigError {
    fn from_config(error: ConfigError) -> Self {
        let code = match error.code {
            ConfigErrorCode::ReadFailed => ProjectConfigErrorCode::ReadFailed,
            ConfigErrorCode::OutsideBoundary => ProjectConfigErrorCode::OutsideBoundary,
            ConfigErrorCode::InvalidToml => ProjectConfigErrorCode::InvalidToml,
            ConfigErrorCode::UnsupportedSchema => ProjectConfigErrorCode::UnsupportedSchema,
            ConfigErrorCode::InvalidRule => ProjectConfigErrorCode::InvalidRule,
            ConfigErrorCode::InvalidAttribute => ProjectConfigErrorCode::InvalidAttribute,
            ConfigErrorCode::InvalidLimit => ProjectConfigErrorCode::InvalidLimit,
            ConfigErrorCode::InvalidPath => ProjectConfigErrorCode::InvalidPath,
            ConfigErrorCode::InvalidRole => ProjectConfigErrorCode::InvalidRole,
        };
        Self {
            code,
            field: error.field.clone(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ProjectConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectConfigError {}

/// Stable category for a project resource failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectResourceErrorCode {
    Missing,
    PermissionDenied,
    InvalidUtf8,
    OutsideAuthority,
    InvalidPath,
    ReadFailed,
    Limit,
    Unverifiable,
}

/// Project-owned resource failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectResourceError {
    pub code: ProjectResourceErrorCode,
    pub path: Option<PathBuf>,
    message: String,
    host: ResourceError,
}

impl ProjectResourceError {
    pub(crate) fn from_host(error: ResourceError) -> Self {
        let (code, path) = match &error {
            ResourceError::Missing(path) => (ProjectResourceErrorCode::Missing, Some(path.clone())),
            ResourceError::PermissionDenied(path) => (
                ProjectResourceErrorCode::PermissionDenied,
                Some(path.clone()),
            ),
            ResourceError::InvalidUtf8 { path, .. } => {
                (ProjectResourceErrorCode::InvalidUtf8, Some(path.clone()))
            }
            ResourceError::OutsideRoots(path) => (
                ProjectResourceErrorCode::OutsideAuthority,
                Some(path.clone()),
            ),
            ResourceError::PathNotAbsolute(path) => {
                (ProjectResourceErrorCode::InvalidPath, Some(path.clone()))
            }
            ResourceError::Read { path, .. } => {
                (ProjectResourceErrorCode::ReadFailed, Some(path.clone()))
            }
            ResourceError::Inspect { path, .. } | ResourceError::NotRegularFile(path) => {
                (ProjectResourceErrorCode::ReadFailed, Some(path.clone()))
            }
            ResourceError::FileLimit { .. }
            | ResourceError::ByteLimit
            | ResourceError::ResourceTooLarge(_)
            | ResourceError::Job(_) => (ProjectResourceErrorCode::Limit, None),
            ResourceError::Unverifiable(_) => (ProjectResourceErrorCode::Unverifiable, None),
            ResourceError::NoRoots
            | ResourceError::InvalidRoot
            | ResourceError::RootLimit { .. } => (ProjectResourceErrorCode::OutsideAuthority, None),
            ResourceError::InvalidSourceId
            | ResourceError::SessionIdentityExhausted
            | ResourceError::ScanEntryLimit { .. } => (ProjectResourceErrorCode::Limit, None),
        };
        Self {
            code,
            path,
            message: error.to_string(),
            host: error,
        }
    }

    pub(crate) fn host(&self) -> &ResourceError {
        &self.host
    }
}

impl fmt::Display for ProjectResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectResourceError {}

/// Effective project configuration owned by this crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfig {
    inner: ResolvedProjectConfig,
}

/// Effective retained-resource ceilings from project configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectResourceLimits {
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub max_resource_bytes: u64,
}

impl ProjectConfig {
    pub fn schema_version(&self) -> u32 {
        self.inner.schema_version
    }
    pub fn analysis(&self) -> &AnalysisOptions {
        &self.inner.analysis
    }
    pub fn preprocess(&self) -> &PreprocessOptions {
        &self.inner.preprocess
    }
    pub fn include_enabled(&self) -> bool {
        self.inner.resources.include
    }
    pub fn resource_roots(&self) -> &[PathBuf] {
        &self.inner.resources.roots
    }
    pub fn resource_limits(&self) -> ProjectResourceLimits {
        let limits = self.inner.resources.limit_plan.filesystem_reads;
        ProjectResourceLimits {
            max_files: limits.max_files,
            max_total_bytes: limits.max_total_bytes,
            max_resource_bytes: limits.max_resource_bytes,
        }
    }
    pub fn workspace_excludes(&self) -> impl Iterator<Item = &str> {
        self.inner.workspace.scan.exclude_patterns()
    }
    pub fn local_targets_enabled(&self) -> bool {
        self.inner.local_targets.enabled
    }
    pub fn local_target_root(&self) -> Option<&Path> {
        self.inner.local_targets.project_root.as_deref()
    }
    pub fn format(&self) -> &FormatConfig {
        &self.inner.format
    }
    pub fn format_newline_explicit(&self) -> bool {
        self.inner.format_newline_explicit
    }
    pub fn format_final_newline_explicit(&self) -> bool {
        self.inner.format_final_newline_explicit
    }
    pub fn html_policy(&self) -> &RenderPolicy {
        &self.inner.html.policy
    }
    pub fn stylesheet_files(&self) -> &[PathBuf] {
        &self.inner.html.stylesheet_files
    }
    pub fn stylesheet_urls(&self) -> &[String] {
        &self.inner.html.stylesheet_urls
    }
}

/// Content-addressed effective configuration for one project scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigSnapshot {
    pub source_id: Option<SourceId>,
    pub path: Option<PathBuf>,
    pub content_sha256: Option<[u8; 32]>,
    pub config: ProjectConfig,
}

impl ProjectConfigSnapshot {
    pub(crate) fn from_resolved(
        snapshot: Option<&ConfigSnapshot>,
        config: &ResolvedProjectConfig,
        source_id: Option<SourceId>,
    ) -> Self {
        Self {
            source_id,
            path: snapshot.map(|value| value.path.clone()),
            content_sha256: snapshot.map(|value| value.content_sha256),
            config: ProjectConfig {
                inner: config.clone(),
            },
        }
    }
}

impl fmt::Display for ProjectTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => error.fmt(formatter),
            Self::Analysis(error) => error.fmt(formatter),
            Self::Incomplete(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProjectTargetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Analysis(error) => Some(error),
            Self::Incomplete(error) => Some(error),
        }
    }
}
