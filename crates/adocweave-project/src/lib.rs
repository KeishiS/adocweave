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
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adocweave::output::diagnostics::LintRuleId;
use adocweave::output::formatter::FormatConfig;
use adocweave::output::html::RenderPolicy;
use adocweave::preprocess::PreprocessOptions;
use adocweave::preprocess::{
    AnalysisProjection, PreprocessError, PreprocessedAnalysis, ProcessingOptionsError,
    ProjectionError,
};
use adocweave::{Analysis, AnalysisOptions, ParseError, SourceId};
use adocweave_config::{ConfigError, ConfigErrorCode, ConfigSnapshot, ResolvedProjectConfig};
use adocweave_host::{
    DerivedFilesystemRoots, FilesystemReadLimits, FilesystemReadOutcome, LocalFilesystemPolicy,
    LocalFilesystemSession, LocalTargetError, LocalTargetPolicy, LogicalSourceId, ResourceError,
};

pub use process::{process, resolve_config};

pub(crate) fn project_authority_error(error: ResourceError) -> ProjectError {
    ProjectError::Authority(ProjectResourceError::from_host(error))
}

pub(crate) fn project_config_error(
    path: PathBuf,
    source: &[u8],
    error: ConfigError,
) -> ProjectError {
    ProjectError::Config(ProjectConfigError::from_config(path, source, error))
}

pub(crate) fn project_target_read(error: ResourceError) -> ProjectTargetError {
    ProjectTargetError::Read(ProjectResourceError::from_host(error))
}

pub(crate) fn project_expansion_read(error: ResourceError) -> ProjectExpansionError {
    ProjectExpansionError::Resource(ProjectResourceError::from_host(error))
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
///
/// A live caller which needs to compare returned observations after this
/// request must retain [`ProjectAuthority::observation_access`] before passing
/// the request to [`process`].
#[derive(Debug)]
pub struct ProjectRequest {
    pub targets: Vec<ProjectTarget>,
    /// In-memory sources available as primary documents or include overlays.
    pub sources: Vec<ProjectSource>,
    pub config: ConfigSelection,
    pub overrides: ProjectOverrides,
    /// Applies diagnostics edits marked as always safe before project analysis.
    pub apply_safe_fixes: bool,
    /// Related resources which this caller needs in the returned result.
    pub resource_selection: ProjectResourceSelection,
    pub authority: ProjectAuthority,
    pub limits: ProjectLimits,
}

/// Related resources and checks required by the caller.
///
/// Includes remain controlled by the resolved configuration. Local-target
/// selection also allows include failures to be reported as local-target
/// diagnostics while analysis continues.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectResourceSelection {
    pub local_targets: bool,
    pub stylesheets: bool,
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
        let filesystem_limits = ProjectLimits::default().filesystem_reads();
        let mut policy = LocalFilesystemPolicy::new(roots, filesystem_limits)
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
                    filesystem_limits,
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

    /// Clones the already opened filesystem authority for caller-owned change
    /// observation. The clone retains the same directory handles and does not
    /// widen the permitted roots.
    pub fn observation_access(&self) -> ProjectObservationAccess {
        ProjectObservationAccess {
            policy: self.policy.clone(),
        }
    }

    pub(crate) fn into_parts(self) -> (PathBuf, LocalFilesystemPolicy) {
        (self.project_root, self.policy)
    }
}

/// Retained filesystem access for caller-owned change observation.
#[derive(Clone, Debug)]
pub struct ProjectObservationAccess {
    policy: LocalFilesystemPolicy,
}

impl ProjectObservationAccess {
    /// Starts one observation pass with one budget shared by every resource.
    pub fn session(&self) -> Result<ProjectObservationSession, ProjectResourceError> {
        let remaining = self.policy.limits().max_files;
        let filesystem = self
            .policy
            .session()
            .map_err(ProjectResourceError::from_host)?;
        Ok(ProjectObservationSession {
            policy: self.policy.clone(),
            filesystem,
            remaining,
            next_source: 0,
        })
    }
}

/// Type of state needed to detect a later project resource change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectObservationKind {
    Contents,
    ContentsNoSymlinks,
    Existence,
}

/// One bounded caller-owned observation pass.
pub struct ProjectObservationSession {
    policy: LocalFilesystemPolicy,
    filesystem: LocalFilesystemSession,
    remaining: usize,
    next_source: usize,
}

impl ProjectObservationSession {
    /// Observes one path without granting access beyond the retained authority.
    pub fn observe(
        &mut self,
        path: &Path,
        kind: ProjectObservationKind,
    ) -> ProjectResourceObservation {
        if self.remaining == 0 {
            return ProjectResourceObservation::unavailable();
        }
        self.remaining -= 1;
        let source_id = LogicalSourceId::new(format!("project-observation:{}", self.next_source));
        self.next_source = self.next_source.saturating_add(1);
        let Ok(source_id) = source_id else {
            return ProjectResourceObservation::unavailable();
        };
        match kind {
            ProjectObservationKind::Existence => self.policy.policy_for_path(path).map_or(
                ProjectResourceObservation::unavailable(),
                |policy| match policy.inspect_candidate_no_symlinks(path) {
                    Ok(_) => ProjectResourceObservation::present(),
                    Err(LocalTargetError::Missing(_)) => ProjectResourceObservation::missing(),
                    Err(_) => ProjectResourceObservation::unavailable(),
                },
            ),
            ProjectObservationKind::Contents | ProjectObservationKind::ContentsNoSymlinks => {
                let result = if kind == ProjectObservationKind::ContentsNoSymlinks {
                    self.filesystem
                        .read_utf8_no_symlinks_outcome(source_id, path)
                } else {
                    self.filesystem.read_utf8_outcome(source_id, path)
                };
                match result {
                    Ok(FilesystemReadOutcome::Found(source)) => {
                        ProjectResourceObservation::from_bytes(source.source().as_bytes())
                    }
                    Ok(FilesystemReadOutcome::NotFound { .. }) => {
                        ProjectResourceObservation::missing()
                    }
                    Err(_) => ProjectResourceObservation::unavailable(),
                }
            }
        }
    }
}

/// A target selector independent of command-line argument types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectTarget {
    /// One source supplied in [`ProjectRequest::sources`].
    Source(SourceId),
    /// One authored file path.
    Path(PathBuf),
    /// One authored file path which must not contain symbolic links.
    PathNoSymlinks(PathBuf),
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
    /// Lint rules enabled for this request in addition to project settings.
    pub enable_lint_rules: Vec<LintRuleId>,
    /// Include roots which replace project settings for this request.
    pub resource_roots: Option<Vec<PathBuf>>,
    /// Local-target root which enables local-target checks for this request.
    pub local_target_project_root: Option<PathBuf>,
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

impl Default for ProjectLimits {
    fn default() -> Self {
        let filesystem = FilesystemReadLimits::default();
        Self {
            // A request can span several independently configured projects.
            // Keep the request-wide ceiling above the per-project default so
            // each resolved configuration remains the practical limit.
            max_files: filesystem.max_files.saturating_mul(10),
            max_resource_bytes: filesystem.max_resource_bytes,
            max_read_bytes: filesystem.max_total_bytes,
            max_directory_entries: 100_000,
            max_processing_iterations: 100_000,
            max_output_bytes: adocweave::OutputLimits::default().max_output_bytes,
        }
    }
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
/// are stored in [`ProjectTargetResult::analysis`] instead.
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
    /// Complete replacement text after safe fixes, when the source changed.
    pub replacement_source: Option<Arc<str>>,
    /// Authority retained from the successful primary read for a later safe
    /// replacement. In-memory inputs and failed reads do not provide it.
    pub write: Option<ProjectWriteCapability>,
    /// Effective configuration and its optional source identity.
    pub config: Arc<ProjectConfigSnapshot>,
    /// Files read or inspected on behalf of this target.
    pub resources: Vec<ProjectResourceResult>,
    /// Primary-source analysis, or the failure which prevents publishing it.
    ///
    /// A successful value can still contain an unsuccessful include expansion
    /// in [`ProjectAnalysis::expanded`].
    pub analysis: Result<ProjectAnalysis, ProjectTargetError>,
}

/// Opaque authority for replacing one file observed by project processing.
#[derive(Debug)]
pub struct ProjectWriteCapability {
    path: PathBuf,
    policy: LocalTargetPolicy,
    original: Arc<str>,
}

impl ProjectWriteCapability {
    pub(crate) fn new(path: PathBuf, policy: LocalTargetPolicy, original: Arc<str>) -> Self {
        Self {
            path,
            policy,
            original,
        }
    }

    /// Returns the file path bound to this capability.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns whether the file still has the contents observed by processing.
    pub fn contents_match(&self) -> Result<bool, ProjectResourceError> {
        self.policy
            .candidate_contents_match(&self.path, self.original.as_bytes())
            .map_err(ResourceError::from)
            .map_err(ProjectResourceError::from_host)
    }

    /// Rechecks and replaces the observed file, consuming the capability.
    ///
    /// `Ok(false)` means that the file changed after project processing and was
    /// not replaced.
    pub fn replace_after_recheck(self, replacement: &[u8]) -> Result<bool, ProjectResourceError> {
        self.policy
            .replace_candidate_after_recheck(&self.path, self.original.as_bytes(), replacement)
            .map_err(ResourceError::from)
            .map_err(ProjectResourceError::from_host)
    }
}

/// Analysis of one primary source and the result of expanding its includes.
#[derive(Debug)]
pub struct ProjectAnalysis {
    /// Analysis of the unexpanded primary source.
    pub primary: Analysis,
    /// Analysis which requires include expansion and source-position mapping.
    ///
    /// When this is unsuccessful, [`Self::primary`] remains valid and can be
    /// used independently.
    pub expanded: Result<ProjectExpandedAnalysis, ProjectExpansionError>,
}

/// Analysis derived from one successful include expansion.
#[derive(Debug)]
pub struct ProjectExpandedAnalysis {
    /// Analysis of the include-expanded document and its source map.
    pub preprocessed: PreprocessedAnalysis,
    /// Editor-facing facts mapped back to positions in the original sources.
    pub source_mapping: AnalysisProjection,
    /// Local-reference failures mapped to each original source occurrence.
    pub local_target_diagnostics: Vec<ProjectLocalTargetDiagnostic>,
}

/// One local-reference diagnostic ready for caller-specific presentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectLocalTargetDiagnostic {
    pub diagnostic: adocweave::output::diagnostics::Diagnostic,
    pub source_id: SourceId,
    pub target: String,
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
    pub kind: ProjectResourceKind,
    pub origin: ProjectResourceOrigin,
    /// Source which requested this resource, including each include edge.
    pub requested_by: Option<SourceId>,
    /// Safely repeatable observation made during this request.
    ///
    /// The path, observation method and acquired state are kept together so a
    /// live caller does not need to infer how to detect a later change.
    pub observation: Option<ProjectObservationCandidate>,
    pub outcome: ProjectResourceOutcome,
}

/// Comparable state observed while acquiring one project resource.
///
/// Live callers can compare a later filesystem observation with this value
/// without rereading a resource between acquisition and result publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectResourceObservation {
    state: ProjectResourceObservationState,
    len: u64,
    content_hash: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectResourceObservationState {
    Content,
    Present,
    Missing,
    Unavailable,
}

impl ProjectResourceObservation {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self {
            state: ProjectResourceObservationState::Content,
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            content_hash: hasher.finish(),
        }
    }

    pub const fn present() -> Self {
        Self {
            state: ProjectResourceObservationState::Present,
            len: 0,
            content_hash: 0,
        }
    }

    pub const fn missing() -> Self {
        Self {
            state: ProjectResourceObservationState::Missing,
            len: 0,
            content_hash: 0,
        }
    }

    pub const fn unavailable() -> Self {
        Self {
            state: ProjectResourceObservationState::Unavailable,
            len: 0,
            content_hash: 0,
        }
    }

    pub(crate) fn from_outcome(outcome: &ProjectResourceOutcome) -> Self {
        match outcome {
            ProjectResourceOutcome::Loaded { source } => Self::from_bytes(source.as_bytes()),
            ProjectResourceOutcome::Present => Self::present(),
            ProjectResourceOutcome::Missing => Self::missing(),
            ProjectResourceOutcome::Failed(_) | ProjectResourceOutcome::LoadedOmitted { .. } => {
                Self::unavailable()
            }
        }
    }
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

impl ProjectError {
    /// Returns state safely observed during this request whose later change
    /// may repair the error. Live callers must have retained observation access
    /// from the request authority before calling [`process`].
    pub fn repair_candidate(&self) -> Option<&ProjectObservationCandidate> {
        match self {
            Self::Config(error) => Some(error.repair_candidate.as_ref()),
            Self::Authority(error) => error.repair_candidate.as_deref(),
            Self::TargetSelection(_) | Self::Cancelled | Self::InvalidInput(_) | Self::Limit(_) => {
                None
            }
        }
    }

    pub(crate) fn with_repair_candidate(mut self, candidate: ProjectObservationCandidate) -> Self {
        if let Self::Authority(error) = &mut self {
            error.repair_candidate = Some(Box::new(candidate));
        }
        self
    }
}

/// One safely acquired state which a live caller may observe again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectObservationCandidate {
    pub path: PathBuf,
    pub kind: ProjectObservationKind,
    pub observation: ProjectResourceObservation,
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
    Parse(ProjectParseError),
    /// Safe edits selected from one source overlap or otherwise conflict.
    EditConflict(String),
    /// The result is incomplete and must not be presented as fully analyzed.
    Incomplete(ProjectLimit),
}

/// Failure after the primary source was read and analyzed successfully.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectExpansionError {
    Resource(ProjectResourceError),
    Options(ProcessingOptionsError),
    Preprocess(PreprocessError),
    Parse(ProjectParseError),
    Projection(ProjectionError),
    /// The expanded result is incomplete and must not be presented as complete.
    Incomplete(ProjectLimit),
}

/// Non-cancellation failure while parsing one source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectParseError {
    LimitExceeded {
        resource: &'static str,
        limit: u32,
        actual: u64,
    },
    Position(adocweave::text::PositionError),
    UnsupportedSyntax,
    InternalInvariant,
}

impl ProjectParseError {
    pub(crate) fn from_parse(error: ParseError) -> Option<Self> {
        match error {
            ParseError::LimitExceeded {
                resource,
                limit,
                actual,
            } => Some(Self::LimitExceeded {
                resource,
                limit,
                actual,
            }),
            ParseError::Position(error) => Some(Self::Position(error)),
            ParseError::UnsupportedSyntax => Some(Self::UnsupportedSyntax),
            ParseError::InternalInvariant => Some(Self::InternalInvariant),
            ParseError::Cancelled => None,
        }
    }
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
    /// Configuration file which produced this error.
    pub path: PathBuf,
    pub field: Option<String>,
    repair_candidate: Box<ProjectObservationCandidate>,
    message: String,
}

impl ProjectConfigError {
    fn from_config(path: PathBuf, source: &[u8], error: ConfigError) -> Self {
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
            path: path.clone(),
            field: error.field.clone(),
            repair_candidate: Box::new(ProjectObservationCandidate {
                path,
                kind: ProjectObservationKind::ContentsNoSymlinks,
                observation: ProjectResourceObservation::from_bytes(source),
            }),
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
    repair_candidate: Option<Box<ProjectObservationCandidate>>,
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
            repair_candidate: None,
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
            Self::Parse(error) => error.fmt(formatter),
            Self::EditConflict(message) => formatter.write_str(message),
            Self::Incomplete(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProjectTargetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::EditConflict(_) => None,
            Self::Incomplete(error) => Some(error),
        }
    }
}

impl fmt::Display for ProjectParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded {
                resource,
                limit,
                actual,
            } => write!(
                formatter,
                "{resource} limit exceeded (limit {limit}, actual {actual})"
            ),
            Self::Position(error) => error.fmt(formatter),
            Self::UnsupportedSyntax => {
                formatter.write_str("unsupported syntax is forbidden in strict mode")
            }
            Self::InternalInvariant => formatter.write_str("internal parsing invariant failed"),
        }
    }
}

impl std::error::Error for ProjectParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Position(error) => Some(error),
            Self::LimitExceeded { .. } | Self::UnsupportedSyntax | Self::InternalInvariant => None,
        }
    }
}

impl fmt::Display for ProjectExpansionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource(error) => error.fmt(formatter),
            Self::Options(error) => error.fmt(formatter),
            Self::Preprocess(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
            Self::Projection(error) => error.fmt(formatter),
            Self::Incomplete(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProjectExpansionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Resource(error) => Some(error),
            Self::Options(error) => Some(error),
            Self::Preprocess(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::Projection(error) => Some(error),
            Self::Incomplete(error) => Some(error),
        }
    }
}
