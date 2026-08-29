//! Owned contracts for processing one project request.
//!
//! A [`ProjectRequest`] contains every target selected for one run together
//! with its configuration choice, verified filesystem authority and finite
//! limits. All values are owned: callers may move a request between their own
//! short-lived processing steps without keeping CLI arguments, LSP messages or
//! borrowed paths alive.
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
use std::path::PathBuf;
use std::sync::Arc;

use adocweave::OutputLimits;
use adocweave::preprocess::{PreprocessedAnalysis, PreprocessedAnalysisError};
use adocweave_config::{ConfigError, ConfigSnapshot, ResolvedProjectConfig};
use adocweave_host::{
    FilesystemJobUsage, FilesystemReadLimits, LocalFilesystemPolicy, LogicalSourceId, ResourceError,
};

pub use process::process;

/// One owned request covering every target selected for a single run.
///
/// `authority` contains already verified directory handles and is the maximum
/// filesystem scope available to the request. Paths read from configuration
/// may narrow this scope but must never widen it. `project_root` is kept
/// separately because it is the boundary for configuration discovery and
/// target interpretation, not an additional grant of filesystem access.
#[derive(Debug)]
pub struct ProjectRequest {
    pub project_root: PathBuf,
    pub targets: Vec<ProjectTarget>,
    pub config: ConfigSelection,
    pub overrides: ProjectOverrides,
    pub authority: LocalFilesystemPolicy,
    pub limits: ProjectLimits,
}

/// A target selector independent of command-line argument types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectTarget {
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

/// How a request selects its project configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ConfigSelection {
    /// Searches from each target towards `ProjectRequest::project_root`.
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectOverrides {
    pub include: Option<bool>,
}

/// Hard ceilings shared by every target in one request.
///
/// Include depth, include count and parser limits remain part of the resolved
/// project configuration. They are deliberately not duplicated here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectLimits {
    pub filesystem_reads: FilesystemReadLimits,
    pub max_directory_entries: u64,
    pub max_processing_iterations: u32,
    /// Maximum UTF-8 bytes retained in returned expanded documents and loaded
    /// resource bodies. Rendered products added by later stages have their own
    /// output accounting and are not charged here.
    pub output: OutputLimits,
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
    /// Recoverable request-wide conditions which leave partial results usable.
    pub warnings: Vec<ProjectWarning>,
    /// Resource use shared by all targets in this request.
    pub usage: ProjectUsage,
}

/// Result for one concrete document selected from the request.
#[derive(Debug)]
pub struct ProjectTargetResult {
    pub source_id: LogicalSourceId,
    pub path: PathBuf,
    /// Exact configuration used for this target, or `None` for defaults.
    pub config: Option<ConfigSnapshot>,
    /// Configuration after applying request-local overrides.
    pub resolved_config: ResolvedProjectConfig,
    /// Files read or inspected on behalf of this target.
    pub resources: Vec<ProjectResourceResult>,
    pub outcome: Result<PreprocessedAnalysis, ProjectTargetError>,
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
    pub source_id: LogicalSourceId,
    pub path: PathBuf,
    pub kind: ProjectResourceKind,
    pub requested_by: Option<LogicalSourceId>,
    pub outcome: ProjectResourceOutcome,
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
    Unreadable(ResourceError),
    Rejected(ResourceError),
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
    pub filesystem: FilesystemJobUsage,
    pub processing_iterations: u32,
    /// Expanded document and loaded resource bytes retained in this result.
    pub output_bytes: u64,
}

/// A safety ceiling reached while processing a request or target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectLimit {
    Files { limit: usize },
    ReadBytes { limit: u64 },
    DirectoryEntries { limit: u64 },
    ProcessingIterations { limit: u32 },
    OutputBytes { limit: u32 },
}

impl fmt::Display for ProjectLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Files { limit } => write!(formatter, "project file limit exceeded: {limit}"),
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
    /// Local-reference projection could not produce verifiable candidates.
    LocalTargetProjection { message: String },
}

/// A malformed target selector known before reading a selected document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetSelectionError {
    InvalidGlob { pattern: String },
}

impl fmt::Display for TargetSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGlob { pattern } => write!(formatter, "invalid target glob: {pattern}"),
        }
    }
}

impl std::error::Error for TargetSelectionError {}

/// Failure which prevents a coherent result for the entire request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectError {
    Config(ConfigError),
    TargetSelection(TargetSelectionError),
    Authority(ResourceError),
    Limit(ProjectLimit),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::TargetSelection(error) => error.fmt(formatter),
            Self::Authority(error) => error.fmt(formatter),
            Self::Limit(error) => error.fmt(formatter),
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
        }
    }
}

/// Failure confined to one selected document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectTargetError {
    Read(ResourceError),
    Analysis(PreprocessedAnalysisError),
    /// The result is incomplete and must not be presented as fully analyzed.
    Incomplete(ProjectLimit),
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
