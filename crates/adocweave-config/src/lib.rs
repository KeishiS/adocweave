//! Strict, versioned project configuration shared by AdocWeave consumers.
//!
//! Parsing a project file never grants filesystem or network authority. The
//! resolved paths and limits remain inputs to a host policy that must restrict
//! them to an independently trusted workspace boundary.
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use adocweave::output::diagnostics::{LintConfig, RuleSettings, Severity, lint_rule};
use adocweave::output::formatter::{FormatConfig, NewlineStyle};
use adocweave::output::html::{HtmlDocumentMode, RenderPolicy};
use adocweave::preprocess::PreprocessOptions;
use adocweave::{AnalysisOptions, SyntaxMode};
use adocweave_host::{
    FilesystemReadLimits, LoadedFilesystemSource, LoadedLocalTarget, LocalTargetError,
    LocalTargetPolicy, LocalTargetSession,
};
use adocweave_workspace::RetainedResourceLimits;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod schema;

/// Conventional project configuration filename.
pub const FILE_NAME: &str = ".adocweave.toml";

/// The range one project file governs.
///
/// Two documents share a scope when the same project file applies to both from
/// the same workspace root. The root is part of the identity because two roots
/// without a project file each get the default settings, and charging their
/// resources to one budget would let one root exhaust the other's.
///
/// The command-line interface and the Language Server both group resources this
/// way, so the identity lives beside the configuration it names rather than
/// once in each of them.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectScopeId {
    /// Workspace root the document was reached from.
    pub workspace_root: PathBuf,
    /// Project file that applies, or `None` when the scope uses the defaults.
    pub config_path: Option<PathBuf>,
}

impl ProjectScopeId {
    /// Names the scope a document belongs to.
    pub fn new(workspace_root: PathBuf, snapshot: Option<&ConfigSnapshot>) -> Self {
        Self {
            workspace_root,
            config_path: snapshot.map(|snapshot| snapshot.path.clone()),
        }
    }
}

/// Largest project file that is read.
///
/// A project file names roots, limits and rule settings. One megabyte is far
/// beyond any of that, so the bound rejects a file that is not a project file
/// without constraining a real one.
pub const MAX_PROJECT_FILE_BYTES: u64 = 1024 * 1024;
/// Configuration schema version accepted by this package.
pub const SCHEMA_VERSION: u32 = 2;

/// Stable category for configuration failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigErrorCode {
    /// Configuration or search path could not be read safely.
    ReadFailed,
    /// Search start lies outside its trusted boundary.
    OutsideBoundary,
    /// TOML is malformed or contains an unknown field.
    InvalidToml,
    /// Schema version is not supported.
    UnsupportedSchema,
    /// Lint rule identifier is unknown.
    InvalidRule,
    /// External attribute has an ambiguous value.
    InvalidAttribute,
    /// Configured processing limit is invalid.
    InvalidLimit,
    /// Configured path is absolute or escapes its configuration directory.
    InvalidPath,
    /// Configured HTML role is not a class token.
    InvalidRole,
}

impl ConfigErrorCode {
    /// Returns the stable kebab-case code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadFailed => "read-failed",
            Self::OutsideBoundary => "outside-boundary",
            Self::InvalidToml => "invalid-toml",
            Self::UnsupportedSchema => "unsupported-schema",
            Self::InvalidRule => "invalid-rule",
            Self::InvalidAttribute => "invalid-attribute",
            Self::InvalidLimit => "invalid-limit",
            Self::InvalidPath => "invalid-path",
            Self::InvalidRole => "invalid-role",
        }
    }
}

/// Configuration error that never contains authored attribute values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError {
    /// Stable error category.
    pub code: ConfigErrorCode,
    /// Schema field associated with the error, when available.
    pub field: Option<String>,
    message: &'static str,
}

impl ConfigError {
    const fn new(code: ConfigErrorCode, message: &'static str) -> Self {
        Self {
            code,
            field: None,
            message,
        }
    }

    fn at(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(field) = &self.field {
            write!(
                formatter,
                "configuration {} at {field}: {}",
                self.code.as_str(),
                self.message
            )
        } else {
            write!(
                formatter,
                "configuration {}: {}",
                self.code.as_str(),
                self.message
            )
        }
    }
}

impl Error for ConfigError {}

/// One immutable, content-addressed view of a project configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigSnapshot {
    /// Canonical path of the selected configuration.
    pub path: PathBuf,
    /// SHA-256 digest of the exact UTF-8 configuration content.
    pub content_sha256: [u8; 32],
    /// Fully resolved typed configuration.
    pub config: ResolvedProjectConfig,
}

impl ConfigSnapshot {
    /// Loads an explicitly selected project configuration.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::load_explicit(path)
    }

    /// Loads an explicit configuration through `preferred` when it belongs to
    /// that retained authority.
    ///
    /// An explicit symbolic link outside the preferred root remains accepted,
    /// but its target receives a separate configuration-only authority.
    pub fn load_with_preferred_policy(
        path: &Path,
        preferred: &LocalTargetPolicy,
    ) -> Result<Self, ConfigError> {
        let candidate = if path.is_absolute() {
            path.to_owned()
        } else {
            preferred.root().join(path)
        };
        match preferred.normalize_candidate(&candidate) {
            Ok(candidate) => {
                let mut session = config_session(preferred.clone(), 1);
                match session.read_candidate_utf8(&candidate) {
                    Ok(loaded) => Self::from_loaded(loaded),
                    Err(LocalTargetError::OutsideRoot(_)) => Self::load_explicit(&candidate),
                    Err(_) => Err(config_read_failed()),
                }
            }
            Err(LocalTargetError::OutsideRoot(_)) => Self::load_explicit(&candidate),
            Err(_) => Err(config_read_failed()),
        }
    }

    fn load_explicit(path: &Path) -> Result<Self, ConfigError> {
        let (_policy, loaded) = LocalTargetPolicy::load_explicit_utf8(path, MAX_PROJECT_FILE_BYTES)
            .map_err(|_| config_read_failed())?;
        Self::from_loaded(loaded)
    }

    fn from_loaded(loaded: LoadedLocalTarget) -> Result<Self, ConfigError> {
        Self::from_source(loaded.canonical_path().to_owned(), loaded.source())
    }

    /// Parses a configuration from a source already read through a retained
    /// filesystem authority.
    pub fn from_filesystem_source(loaded: &LoadedFilesystemSource) -> Result<Self, ConfigError> {
        Self::from_source(loaded.canonical_path().to_owned(), loaded.source())
    }

    /// Parses configuration text whose path and contents were fixed by a
    /// request-scoped host before this call.
    pub fn from_utf8_source(path: PathBuf, source: &str) -> Result<Self, ConfigError> {
        Self::from_source(path, source)
    }

    fn from_source(path: PathBuf, source: &str) -> Result<Self, ConfigError> {
        let directory = path.parent().ok_or_else(|| {
            ConfigError::new(
                ConfigErrorCode::ReadFailed,
                "the project file has no parent directory",
            )
        })?;
        let content_sha256 = Sha256::digest(source.as_bytes()).into();
        let config = ResolvedProjectConfig::parse(source, directory)?;
        Ok(Self {
            path,
            content_sha256,
            config,
        })
    }
}

/// Finds the nearest project file without searching above `boundary`.
///
/// The boundary must exist. A missing `start` searches from its nearest
/// existing parent, while an existing file starts from its parent.
pub fn discover(start: &Path, boundary: &Path) -> Result<Option<PathBuf>, ConfigError> {
    discover_from_search(config_search(start, boundary)?)
}

/// Finds the nearest project file through an already opened root policy.
///
/// The policy's root is the search boundary. The returned path identifies the
/// project file but does not read its contents.
pub fn discover_with_policy(
    start: &Path,
    policy: &LocalTargetPolicy,
) -> Result<Option<PathBuf>, ConfigError> {
    discover_from_search(config_search_with_policy(start, policy.clone())?)
}

fn discover_from_search(mut search: ConfigSearch) -> Result<Option<PathBuf>, ConfigError> {
    loop {
        let candidate = search.directory.join(FILE_NAME);
        match search.policy.inspect_candidate_no_symlinks(&candidate) {
            Ok(path) => return Ok(Some(path)),
            Err(LocalTargetError::Missing(_)) => {}
            Err(_) => return Err(config_read_failed()),
        }
        if search.directory == search.boundary {
            return Ok(None);
        }
        if !search.directory.pop() {
            return Ok(None);
        }
    }
}

fn discover_loaded_with(
    start: &Path,
    boundary: &Path,
    after_policy: impl FnOnce(),
) -> Result<Option<LoadedLocalTarget>, ConfigError> {
    let policy = config_policy(boundary)?;
    discover_loaded_from_policy_with(start, policy, after_policy)
}

fn discover_loaded_from_policy_with(
    start: &Path,
    policy: LocalTargetPolicy,
    after_policy: impl FnOnce(),
) -> Result<Option<LoadedLocalTarget>, ConfigError> {
    let mut search = config_search_with_policy(start, policy)?;
    let mut session = config_session(search.policy, search.max_paths);
    after_policy();

    loop {
        let candidate = search.directory.join(FILE_NAME);
        match session.read_candidate_utf8_no_symlinks(&candidate) {
            Ok(loaded) => return Ok(Some(loaded)),
            Err(LocalTargetError::Missing(_)) => {}
            Err(_) => return Err(config_read_failed()),
        }
        if search.directory == search.boundary {
            return Ok(None);
        }
        if !search.directory.pop() {
            return Ok(None);
        }
    }
}

struct ConfigSearch {
    policy: LocalTargetPolicy,
    boundary: PathBuf,
    directory: PathBuf,
    max_paths: usize,
}

fn config_search(start: &Path, boundary: &Path) -> Result<ConfigSearch, ConfigError> {
    config_search_with_policy(start, config_policy(boundary)?)
}

fn config_policy(boundary: &Path) -> Result<LocalTargetPolicy, ConfigError> {
    LocalTargetPolicy::new(boundary).map_err(|_| {
        ConfigError::new(
            ConfigErrorCode::ReadFailed,
            "the search boundary cannot be resolved",
        )
    })
}

fn config_search_with_policy(
    start: &Path,
    policy: LocalTargetPolicy,
) -> Result<ConfigSearch, ConfigError> {
    let boundary = policy.root().to_owned();
    let mut current = policy
        .normalize_candidate(start)
        .map_err(config_start_error)?;
    let directory = loop {
        if current == boundary {
            break policy
                .inspect_directory_no_symlinks(&current)
                .map_err(config_start_error)?;
        }
        match policy.inspect_candidate(&current) {
            Ok(file) => {
                let parent = file
                    .parent()
                    .expect("a verified file path has a parent")
                    .to_path_buf();
                break policy
                    .inspect_directory_no_symlinks(&parent)
                    .map_err(config_start_error)?;
            }
            Err(LocalTargetError::NotFile(_)) => {
                break policy
                    .inspect_directory_no_symlinks(&current)
                    .map_err(config_start_error)?;
            }
            Err(LocalTargetError::Missing(_)) => {
                if !current.pop() {
                    return Err(config_start_failed());
                }
            }
            Err(error) => return Err(config_start_error(error)),
        }
    };
    if !directory.starts_with(&boundary) {
        return Err(ConfigError::new(
            ConfigErrorCode::OutsideBoundary,
            "the search start is outside the trusted boundary",
        ));
    }

    let max_paths = directory
        .strip_prefix(&boundary)
        .map(|relative| relative.components().count().saturating_add(1))
        .unwrap_or(1);
    Ok(ConfigSearch {
        policy,
        boundary,
        directory,
        max_paths,
    })
}

fn config_start_failed() -> ConfigError {
    ConfigError::new(
        ConfigErrorCode::ReadFailed,
        "the search start cannot be resolved",
    )
}

fn config_start_error(error: LocalTargetError) -> ConfigError {
    if matches!(error, LocalTargetError::OutsideRoot(_)) {
        ConfigError::new(
            ConfigErrorCode::OutsideBoundary,
            "the search start is outside the trusted boundary",
        )
    } else {
        config_start_failed()
    }
}

/// Discovers and loads one project configuration snapshot.
pub fn discover_and_load(
    start: &Path,
    boundary: &Path,
) -> Result<Option<ConfigSnapshot>, ConfigError> {
    discover_loaded_with(start, boundary, || {})?
        .map(ConfigSnapshot::from_loaded)
        .transpose()
}

/// Discovers and loads a project configuration through an already opened root.
///
/// The policy's canonical root is the search boundary. Cloning the policy
/// retains the same directory handle on platforms which provide
/// handle-relative resolution, so configuration and document reads can share
/// one authority even if the root path is concurrently replaced.
pub fn discover_and_load_with_policy(
    start: &Path,
    policy: &LocalTargetPolicy,
) -> Result<Option<ConfigSnapshot>, ConfigError> {
    discover_loaded_from_policy_with(start, policy.clone(), || {})?
        .map(ConfigSnapshot::from_loaded)
        .transpose()
}

fn config_session(policy: LocalTargetPolicy, max_paths: usize) -> LocalTargetSession {
    LocalTargetSession::new(
        policy,
        max_paths,
        FilesystemReadLimits {
            max_files: 1,
            max_total_bytes: MAX_PROJECT_FILE_BYTES,
            max_resource_bytes: MAX_PROJECT_FILE_BYTES,
        },
    )
}

fn config_read_failed() -> ConfigError {
    ConfigError::new(
        ConfigErrorCode::ReadFailed,
        "the project file cannot be read safely",
    )
}

/// Stable category for analysis-snapshot resource limit failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisSnapshotLimitError {
    /// Effective resource count exceeds the configured limit.
    ResourceCount,
    /// One effective resource exceeds the configured byte limit.
    ResourceBytes,
    /// Combined effective resource bytes exceed the configured limit.
    TotalBytes,
}

impl fmt::Display for AnalysisSnapshotLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResourceCount => "analysis snapshot resource count limit exceeded",
            Self::ResourceBytes => "analysis snapshot single-resource byte limit exceeded",
            Self::TotalBytes => "analysis snapshot total byte limit exceeded",
        })
    }
}

impl Error for AnalysisSnapshotLimitError {}

/// Transactional counter shared by adapters which build one analysis snapshot.
#[derive(Clone, Copy, Debug)]
pub struct AnalysisSnapshotBudget {
    limits: adocweave_workspace::RetainedResourceLimits,
    resources: usize,
    bytes: u64,
}

impl AnalysisSnapshotBudget {
    /// Starts an empty budget with resolved limits.
    pub const fn new(limits: adocweave_workspace::RetainedResourceLimits) -> Self {
        Self {
            limits,
            resources: 0,
            bytes: 0,
        }
    }

    /// Charges one effective logical resource before it enters the snapshot.
    pub fn charge(&mut self, bytes: u64) -> Result<(), AnalysisSnapshotLimitError> {
        if bytes > self.limits.max_resource_bytes {
            return Err(AnalysisSnapshotLimitError::ResourceBytes);
        }
        let resources = self
            .resources
            .checked_add(1)
            .ok_or(AnalysisSnapshotLimitError::ResourceCount)?;
        if resources > self.limits.max_files {
            return Err(AnalysisSnapshotLimitError::ResourceCount);
        }
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or(AnalysisSnapshotLimitError::TotalBytes)?;
        if total > self.limits.max_total_bytes {
            return Err(AnalysisSnapshotLimitError::TotalBytes);
        }
        self.resources = resources;
        self.bytes = total;
        Ok(())
    }

    /// Returns the committed resource count.
    pub const fn resources(self) -> usize {
        self.resources
    }

    /// Returns the committed byte count.
    pub const fn bytes(self) -> u64 {
        self.bytes
    }
}

/// Resource limits resolved once from one document's nearest project file.
///
/// Filesystem reads use the host's limit type, while retained layers and the
/// analysis snapshot share the workspace's. One shared type for all three is
/// impossible because the workspace state machine must not depend on the host
/// crate; what differs between the fields is the budget that enforces them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedResourceLimitPlan {
    /// Limits enforced by the host before and during filesystem reads.
    pub filesystem_reads: FilesystemReadLimits,
    /// Limits enforced before disk or overlay layers enter workspace state.
    pub retained_layers: RetainedResourceLimits,
    /// Limits enforced when selecting effective resources for analysis.
    pub analysis_snapshot: adocweave_workspace::RetainedResourceLimits,
}

impl Default for ResolvedResourceLimitPlan {
    fn default() -> Self {
        Self::from_configured(10_000, 50 * 1024 * 1024, 10 * 1024 * 1024)
    }
}

impl ResolvedResourceLimitPlan {
    const fn from_configured(
        max_files: usize,
        max_total_bytes: u64,
        max_resource_bytes: u64,
    ) -> Self {
        Self {
            filesystem_reads: FilesystemReadLimits {
                max_files,
                max_total_bytes,
                max_resource_bytes,
            },
            retained_layers: RetainedResourceLimits {
                max_files,
                max_total_bytes,
                max_resource_bytes,
            },
            analysis_snapshot: adocweave_workspace::RetainedResourceLimits {
                max_files,
                max_total_bytes,
                max_resource_bytes,
            },
        }
    }
}

/// Include policy and bounded local resource settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceSettings {
    /// Whether include preprocessing is enabled.
    pub include: bool,
    /// Configuration-relative roots proposed to the host policy.
    pub roots: Vec<PathBuf>,
    /// Resource limits no greater than built-in ceilings.
    pub limit_plan: ResolvedResourceLimitPlan,
}

/// Include preprocessing is on unless a project turns it off.
///
/// `include::` is part of the document, not an extension of it: a manual split
/// across files is one document that happens to live in several. Reading it as
/// separate files produces diagnostics about text that is not there. What the
/// setting still decides is the reachable set, through `roots` and the host
/// filesystem boundary, so turning this on grants no path that was closed.
impl Default for ResourceSettings {
    fn default() -> Self {
        Self {
            include: true,
            roots: Vec::new(),
            limit_plan: ResolvedResourceLimitPlan::default(),
        }
    }
}

/// Local target validation settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalTargetSettings {
    /// Whether local target validation is enabled.
    pub enabled: bool,
    /// Configuration-relative project root proposed to the host policy.
    pub project_root: Option<PathBuf>,
}

/// Complete-document rendering and stylesheet settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HtmlSettings {
    /// Deterministic HTML rendering policy.
    pub policy: RenderPolicy,
    /// Configuration-relative stylesheet files.
    pub stylesheet_files: Vec<PathBuf>,
    /// Authored stylesheet URLs, subject to the active URL policy.
    pub stylesheet_urls: Vec<String>,
}

/// Fully typed schema-version-2 project configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProjectConfig {
    /// Parsed schema version.
    pub schema_version: u32,
    /// Core syntax, attribute, and diagnostic options.
    pub analysis: AnalysisOptions,
    /// Include preprocessing options sharing analysis attributes and expansion limits.
    pub preprocess: PreprocessOptions,
    /// Local resource settings.
    pub resources: ResourceSettings,
    /// Local target validation settings.
    pub local_targets: LocalTargetSettings,
    /// Formatter settings.
    pub format: FormatConfig,
    /// Whether `format.newline` was present in the project file.
    pub format_newline_explicit: bool,
    /// Whether `format.final-newline` was present in the project file.
    pub format_final_newline_explicit: bool,
    /// HTML and stylesheet settings.
    pub html: HtmlSettings,
}

impl Default for ResolvedProjectConfig {
    fn default() -> Self {
        let resources = ResourceSettings::default();
        let preprocess = PreprocessOptions {
            enable_includes: resources.include,
            ..PreprocessOptions::default()
        };
        Self {
            schema_version: SCHEMA_VERSION,
            analysis: AnalysisOptions::default(),
            preprocess,
            resources,
            local_targets: LocalTargetSettings::default(),
            format: FormatConfig::default(),
            format_newline_explicit: false,
            format_final_newline_explicit: false,
            html: HtmlSettings::default(),
        }
    }
}

impl ResolvedProjectConfig {
    /// Parses strict TOML and resolves relative paths against `directory`.
    pub fn parse(source: &str, directory: &Path) -> Result<Self, ConfigError> {
        let wire: ProjectConfigWire = toml::from_str(source).map_err(|_| {
            ConfigError::new(
                ConfigErrorCode::InvalidToml,
                "the project file is not valid strict TOML",
            )
        })?;
        wire.resolve(directory)
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ProjectConfigWire {
    schema_version: u32,
    #[serde(default)]
    analysis: AnalysisWire,
    #[serde(default)]
    lint: LintWire,
    #[serde(default)]
    resources: ResourcesWire,
    #[serde(default)]
    local_targets: LocalTargetsWire,
    #[serde(default)]
    format: FormatWire,
    #[serde(default)]
    html: HtmlWire,
}

impl ProjectConfigWire {
    fn resolve(self, directory: &Path) -> Result<ResolvedProjectConfig, ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::new(
                ConfigErrorCode::UnsupportedSchema,
                "only schema version 2 is supported",
            )
            .at("schema-version"));
        }

        let mut resolved = ResolvedProjectConfig::default();
        resolved.analysis.syntax.syntax_mode = self.analysis.syntax_mode.into();
        for (name, attribute) in self.analysis.attributes {
            let value = attribute.resolve(&format!("analysis.attributes.{name}"))?;
            resolved.analysis.attributes.insert(name, value);
        }
        resolved.preprocess.attributes = resolved.analysis.attributes.clone();
        resolved.preprocess.max_attribute_expansion_depth = resolved
            .analysis
            .syntax
            .limits
            .max_attribute_expansion_depth;
        resolved.preprocess.max_attribute_expansion_bytes = resolved
            .analysis
            .syntax
            .limits
            .max_attribute_expansion_bytes;
        self.lint.apply(&mut resolved.analysis.diagnostics.lint)?;
        resolved.resources = self.resources.resolve(directory)?;
        resolved.preprocess.enable_includes = resolved.resources.include;
        resolved.preprocess.max_total_bytes = u32::try_from(
            resolved
                .resources
                .limit_plan
                .analysis_snapshot
                .max_total_bytes,
        )
        .map_err(|_| {
            ConfigError::new(ConfigErrorCode::InvalidLimit, "limit exceeds u32")
                .at("resources.max-total-bytes")
        })?;
        resolved.local_targets = self.local_targets.resolve(directory)?;
        resolved.format_newline_explicit = self.format.newline.is_some();
        resolved.format_final_newline_explicit = self.format.final_newline.is_some();
        resolved.format = self.format.resolve()?;
        resolved.html = self.html.resolve(directory)?;
        Ok(resolved)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
enum SyntaxModeWire {
    #[default]
    Permissive,
    Strict,
}

impl From<SyntaxModeWire> for SyntaxMode {
    fn from(value: SyntaxModeWire) -> Self {
        match value {
            SyntaxModeWire::Permissive => Self::Permissive,
            SyntaxModeWire::Strict => Self::Strict,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct AnalysisWire {
    #[serde(default)]
    syntax_mode: SyntaxModeWire,
    #[serde(default)]
    attributes: BTreeMap<String, AttributeWire>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct AttributeWire {
    value: Option<String>,
    unset: Option<bool>,
}

impl AttributeWire {
    fn resolve(self, field: &str) -> Result<Option<String>, ConfigError> {
        match (self.value, self.unset) {
            (Some(value), None) => Ok(Some(value)),
            (None, Some(true)) => Ok(None),
            _ => Err(ConfigError::new(
                ConfigErrorCode::InvalidAttribute,
                "set exactly one of value or unset=true",
            )
            .at(field)),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct LintWire {
    #[serde(default)]
    rules: BTreeMap<String, RuleWire>,
    max_line_length: Option<usize>,
    max_consecutive_blank_lines: Option<usize>,
    max_diagnostics: Option<usize>,
}

impl LintWire {
    fn apply(self, config: &mut LintConfig) -> Result<(), ConfigError> {
        if let Some(value) = self.max_line_length {
            ensure_positive(value, "lint.max-line-length")?;
            config.max_line_length = value;
        }
        if let Some(value) = self.max_consecutive_blank_lines {
            ensure_positive(value, "lint.max-consecutive-blank-lines")?;
            config.max_consecutive_blank_lines = value;
        }
        if let Some(value) = self.max_diagnostics {
            ensure_positive(value, "lint.max-diagnostics")?;
            config.max_diagnostics = value;
        }
        for (name, rule) in self.rules {
            let Some(descriptor) = lint_rule(&name) else {
                return Err(
                    ConfigError::new(ConfigErrorCode::InvalidRule, "unknown lint rule")
                        .at(format!("lint.rules.{name}")),
                );
            };
            let current = config.rule(descriptor.id);
            config.set_rule(
                descriptor.id,
                RuleSettings {
                    enabled: rule.enabled.unwrap_or(current.enabled),
                    severity: rule
                        .severity
                        .map(SeverityWire::into)
                        .unwrap_or(current.severity),
                },
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
enum SeverityWire {
    Error,
    Warning,
    Information,
    Hint,
}

impl From<SeverityWire> for Severity {
    fn from(value: SeverityWire) -> Self {
        match value {
            SeverityWire::Error => Self::Error,
            SeverityWire::Warning => Self::Warning,
            SeverityWire::Information => Self::Information,
            SeverityWire::Hint => Self::Hint,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RuleWire {
    enabled: Option<bool>,
    severity: Option<SeverityWire>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ResourcesWire {
    #[serde(default = "include_preprocessing")]
    include: bool,
    #[serde(default)]
    roots: Vec<ProjectRelativePathWire>,
    max_files: Option<usize>,
    max_total_bytes: Option<u64>,
    max_resource_bytes: Option<u64>,
}

const fn include_preprocessing() -> bool {
    true
}

/// Matches the resolved default, for a project file with no `[resources]`.
impl Default for ResourcesWire {
    fn default() -> Self {
        Self {
            include: include_preprocessing(),
            roots: Vec::new(),
            max_files: None,
            max_total_bytes: None,
            max_resource_bytes: None,
        }
    }
}

impl ResourcesWire {
    fn resolve(self, directory: &Path) -> Result<ResourceSettings, ConfigError> {
        let ceiling = ResolvedResourceLimitPlan::default().filesystem_reads;
        let max_files = bounded(self.max_files, ceiling.max_files, "resources.max-files")?;
        let max_total_bytes = bounded(
            self.max_total_bytes,
            ceiling.max_total_bytes,
            "resources.max-total-bytes",
        )?;
        let max_resource_bytes = bounded(
            self.max_resource_bytes,
            ceiling.max_resource_bytes,
            "resources.max-resource-bytes",
        )?;
        let limit_plan = ResolvedResourceLimitPlan::from_configured(
            max_files,
            max_total_bytes,
            max_resource_bytes,
        );
        if max_resource_bytes > max_total_bytes {
            return Err(ConfigError::new(
                ConfigErrorCode::InvalidLimit,
                "resource limit exceeds the total byte limit",
            )
            .at("resources.max-resource-bytes"));
        }
        let roots = self
            .roots
            .into_iter()
            .enumerate()
            .map(|(index, path)| path.resolve(directory, format!("resources.roots.{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ResourceSettings {
            include: self.include,
            roots,
            limit_plan,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct LocalTargetsWire {
    #[serde(default)]
    enabled: bool,
    project_root: Option<ProjectRelativePathWire>,
}

impl LocalTargetsWire {
    fn resolve(self, directory: &Path) -> Result<LocalTargetSettings, ConfigError> {
        let project_root = self
            .project_root
            .map(|path| path.resolve(directory, "local-targets.project-root"))
            .transpose()?;
        if self.enabled && project_root.is_none() {
            return Err(ConfigError::new(
                ConfigErrorCode::InvalidPath,
                "enabled local target checks require project-root",
            )
            .at("local-targets.project-root"));
        }
        Ok(LocalTargetSettings {
            enabled: self.enabled,
            project_root,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
enum NewlineWire {
    #[default]
    Lf,
    CrLf,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct FormatWire {
    newline: Option<NewlineWire>,
    final_newline: Option<bool>,
    #[serde(default = "default_blank_lines")]
    max_consecutive_blank_lines: usize,
}

impl Default for FormatWire {
    fn default() -> Self {
        Self {
            newline: None,
            final_newline: None,
            max_consecutive_blank_lines: default_blank_lines(),
        }
    }
}

impl FormatWire {
    fn resolve(self) -> Result<FormatConfig, ConfigError> {
        ensure_positive(
            self.max_consecutive_blank_lines,
            "format.max-consecutive-blank-lines",
        )?;
        Ok(FormatConfig {
            newline: match self.newline.unwrap_or_default() {
                NewlineWire::Lf => NewlineStyle::Lf,
                NewlineWire::CrLf => NewlineStyle::CrLf,
            },
            final_newline: self.final_newline.unwrap_or_else(default_true),
            max_consecutive_blank_lines: self.max_consecutive_blank_lines,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct HtmlWire {
    #[serde(default)]
    complete: bool,
    #[serde(default)]
    stylesheet_files: Vec<ProjectRelativePathWire>,
    #[serde(default)]
    stylesheet_urls: Vec<String>,
    #[serde(default)]
    roles: Vec<String>,
}

impl HtmlWire {
    fn resolve(self, directory: &Path) -> Result<HtmlSettings, ConfigError> {
        let mut policy = RenderPolicy {
            document_mode: if self.complete {
                HtmlDocumentMode::Complete
            } else {
                HtmlDocumentMode::Fragment
            },
            ..RenderPolicy::default()
        };
        policy.stylesheets.sources.clear();
        for (index, role) in self.roles.iter().enumerate() {
            if !adocweave::output::html::is_role_name(role) {
                return Err(ConfigError::new(
                    ConfigErrorCode::InvalidRole,
                    "html.roles entries must use ASCII letters, digits, `-`, and `_`",
                )
                .at(format!("html.roles.{index}")));
            }
        }
        policy.roles.allowed = self.roles.into_iter().collect();
        let stylesheet_files = self
            .stylesheet_files
            .into_iter()
            .enumerate()
            .map(|(index, path)| path.resolve(directory, format!("html.stylesheet-files.{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HtmlSettings {
            policy,
            stylesheet_files,
            stylesheet_urls: self.stylesheet_urls,
        })
    }
}

fn default_true() -> bool {
    true
}

fn default_blank_lines() -> usize {
    1
}

fn ensure_positive(value: usize, field: &str) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(
            ConfigError::new(ConfigErrorCode::InvalidLimit, "limit must be positive").at(field),
        );
    }
    Ok(())
}

fn bounded<T>(value: Option<T>, ceiling: T, field: &str) -> Result<T, ConfigError>
where
    T: Copy + Ord + From<u8>,
{
    let value = value.unwrap_or(ceiling);
    if value < T::from(1) || value > ceiling {
        return Err(ConfigError::new(
            ConfigErrorCode::InvalidLimit,
            "limit must be positive and cannot exceed the host ceiling",
        )
        .at(field));
    }
    Ok(value)
}

#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
/// プロジェクトディレクトリの内側を指す、`/`区切りの相対パス。
struct ProjectRelativePathWire(
    #[cfg_attr(
        test,
        schemars(
            with = "String",
            length(min = 1),
            regex(pattern = PROJECT_RELATIVE_PATH_PATTERN)
        )
    )]
    PathBuf,
);

#[cfg(test)]
const PROJECT_RELATIVE_PATH_PATTERN: &str = r"^(?![A-Za-z]:)(?!/)(?!.*(?:^|/)\.\.(?:/|$))[^\\]+$";

impl ProjectRelativePathWire {
    fn resolve(self, directory: &Path, field: impl Into<String>) -> Result<PathBuf, ConfigError> {
        let path = self.0;
        let field = field.into();
        let Some(source) = path.to_str() else {
            return Err(ConfigError::new(
                ConfigErrorCode::InvalidPath,
                "path must use Unicode characters",
            )
            .at(field));
        };
        let has_drive_prefix =
            source.as_bytes().get(1) == Some(&b':') && source.as_bytes()[0].is_ascii_alphabetic();
        if source.is_empty()
            || source.starts_with('/')
            || source.contains('\\')
            || has_drive_prefix
            || source.split('/').any(|component| component == "..")
        {
            return Err(ConfigError::new(
                ConfigErrorCode::InvalidPath,
                "path must stay inside the project directory and use `/` separators",
            )
            .at(field));
        }
        Ok(directory.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "adocweave-config-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove test directory");
        }
    }

    #[test]
    fn strict_project_config_resolves_shared_consumer_options() {
        let config = ResolvedProjectConfig::parse(
            r#"
schema-version = 2

[analysis]
syntax-mode = "strict"

[analysis.attributes.release]
value = "draft"

[analysis.attributes.hidden]
unset = true

[lint.rules.macro-boundary]
enabled = true
severity = "error"

[lint.rules.trailing-whitespace]
enabled = false
severity = "hint"

[resources]
include = true
roots = ["docs"]
max-files = 20
max-total-bytes = 4096
max-resource-bytes = 2048

[local-targets]
enabled = true
project-root = "docs"

[format]
newline = "cr-lf"
final-newline = false
max-consecutive-blank-lines = 2

[html]
complete = true
stylesheet-files = ["styles/manual.css"]
stylesheet-urls = ["https://example.test/manual.css"]
roles = ["definition", "theorem"]
"#,
            Path::new("/workspace"),
        )
        .expect("valid config");

        assert_eq!(config.analysis.syntax.syntax_mode, SyntaxMode::Strict);
        assert_eq!(
            config.analysis.attributes.get("release"),
            Some(&Some("draft".to_owned()))
        );
        assert_eq!(config.analysis.attributes.get("hidden"), Some(&None));
        assert_eq!(config.preprocess.attributes, config.analysis.attributes);
        assert_eq!(
            config.preprocess.max_attribute_expansion_depth,
            config.analysis.syntax.limits.max_attribute_expansion_depth
        );
        assert_eq!(
            config.preprocess.max_attribute_expansion_bytes,
            config.analysis.syntax.limits.max_attribute_expansion_bytes
        );
        assert!(
            config
                .analysis
                .diagnostics
                .lint
                .rule(lint_rule("macro-boundary").expect("known rule").id)
                .enabled
        );
        let trailing = config
            .analysis
            .diagnostics
            .lint
            .rule(lint_rule("trailing-whitespace").expect("known rule").id);
        assert!(!trailing.enabled);
        assert_eq!(trailing.severity, Severity::Hint);
        assert_eq!(config.resources.roots, [PathBuf::from("/workspace/docs")]);
        assert_eq!(
            config.resources.limit_plan,
            ResolvedResourceLimitPlan {
                filesystem_reads: FilesystemReadLimits {
                    max_files: 20,
                    max_total_bytes: 4096,
                    max_resource_bytes: 2048,
                },
                retained_layers: RetainedResourceLimits {
                    max_files: 20,
                    max_total_bytes: 4096,
                    max_resource_bytes: 2048,
                },
                analysis_snapshot: adocweave_workspace::RetainedResourceLimits {
                    max_files: 20,
                    max_total_bytes: 4096,
                    max_resource_bytes: 2048,
                },
            }
        );
        assert_eq!(
            config.local_targets.project_root,
            Some(PathBuf::from("/workspace/docs"))
        );
        assert_eq!(config.format.newline, NewlineStyle::CrLf);
        assert!(!config.format.final_newline);
        assert_eq!(config.html.policy.document_mode, HtmlDocumentMode::Complete);
        assert!(config.html.policy.roles.allows("definition"));
        assert!(config.html.policy.roles.allows("theorem"));
        assert!(!config.html.policy.roles.allows("lemma"));
    }

    /// A role that is not a class token cannot reach a stylesheet, so the
    /// configuration rejects it by field instead of dropping it silently.
    #[test]
    fn include_preprocessing_does_not_depend_on_the_project_file_existing() {
        // The trap this replaces: a project file added for an unrelated setting
        // used to turn include resolution off, because the parsed default said
        // false while a workspace without a project file said true.
        for source in [
            "schema-version = 2\n",
            "schema-version = 2\n[lint]\nmax-line-length = 100\n",
            "schema-version = 2\n[resources]\nroots = [\"docs\"]\n",
            "schema-version = 2\n[resources]\ninclude = true\n",
        ] {
            let config =
                ResolvedProjectConfig::parse(source, Path::new("/workspace")).expect("valid");
            assert!(config.resources.include, "{source}");
            assert!(config.preprocess.enable_includes, "{source}");
        }

        let default = ResolvedProjectConfig::default();
        assert!(default.resources.include);
        assert!(default.preprocess.enable_includes);

        let disabled = ResolvedProjectConfig::parse(
            "schema-version = 2\n[resources]\ninclude = false\n",
            Path::new("/workspace"),
        )
        .expect("valid");
        assert!(!disabled.resources.include);
        assert!(!disabled.preprocess.enable_includes);
    }

    #[test]
    fn html_roles_must_be_class_tokens() {
        let error = ResolvedProjectConfig::parse(
            "schema-version = 2\n[html]\nroles = [\"ok\", \"not ok\"]\n",
            Path::new("/workspace"),
        )
        .expect_err("invalid role");
        assert_eq!(error.code, ConfigErrorCode::InvalidRole);
        assert_eq!(error.field.as_deref(), Some("html.roles.1"));
    }

    #[test]
    fn rejects_unknown_fields_versions_rules_and_ambiguous_attributes() {
        for (source, code) in [
            ("schema-version = 1", ConfigErrorCode::UnsupportedSchema),
            (
                "schema-version = 2\nunknown = true",
                ConfigErrorCode::InvalidToml,
            ),
            (
                "schema-version = 2\n[workspace.scan]\nexclude = []",
                ConfigErrorCode::InvalidToml,
            ),
            (
                "schema-version = 2\n[lint.rules.unknown]\nenabled = true",
                ConfigErrorCode::InvalidRule,
            ),
            (
                "schema-version = 2\n[analysis.attributes.secret]\nvalue = \"x\"\nunset = true",
                ConfigErrorCode::InvalidAttribute,
            ),
            (
                "schema-version = 2\n[analysis.attributes.secret]\nvalue = \"x\"\nunset = false",
                ConfigErrorCode::InvalidAttribute,
            ),
        ] {
            assert_eq!(
                ResolvedProjectConfig::parse(source, Path::new("/workspace"))
                    .expect_err("invalid config")
                    .code,
                code
            );
        }
    }

    #[test]
    fn every_catalog_rule_is_accepted_by_project_configuration() {
        for descriptor in adocweave::output::diagnostics::LINT_RULES {
            let source = format!(
                "schema-version = 2\n[lint.rules.{}]\nenabled = false\nseverity = \"hint\"\n",
                descriptor.id.as_str()
            );
            let config = ResolvedProjectConfig::parse(&source, Path::new("/workspace"))
                .unwrap_or_else(|error| {
                    panic!(
                        "catalog rule {} must be configurable: {error}",
                        descriptor.id.as_str()
                    )
                });
            let settings = config.analysis.diagnostics.lint.rule(descriptor.id);
            assert!(!settings.enabled, "{}", descriptor.id.as_str());
            assert_eq!(
                settings.severity,
                Severity::Hint,
                "{}",
                descriptor.id.as_str()
            );
        }
    }

    #[test]
    fn project_config_cannot_expand_host_authority() {
        for source in [
            "schema-version = 2\n[resources]\nroots = [\"../private\"]",
            "schema-version = 2\n[resources]\nroots = [\"/private\"]",
            "schema-version = 2\n[resources]\nmax-files = 10001",
            "schema-version = 2\n[resources]\nmax-total-bytes = 10\nmax-resource-bytes = 11",
        ] {
            assert!(ResolvedProjectConfig::parse(source, Path::new("/workspace")).is_err());
        }
    }

    #[test]
    fn configured_paths_use_portable_project_relative_syntax() {
        for source in [
            "schema-version = 2\n[resources]\nroots = [\"C:/private\"]",
            "schema-version = 2\n[resources]\nroots = [\"C:\\\\private\"]",
            "schema-version = 2\n[local-targets]\nproject-root = \"\\\\server\\\\share\"",
            "schema-version = 2\n[html]\nstylesheet-files = [\"styles\\\\manual.css\"]",
        ] {
            let error = ResolvedProjectConfig::parse(source, Path::new("/workspace"))
                .expect_err("platform-specific path must be rejected");
            assert_eq!(error.code, ConfigErrorCode::InvalidPath, "{source}");
        }

        let config = ResolvedProjectConfig::parse(
            concat!(
                "schema-version = 2\n",
                "[resources]\nroots = [\"docs/api\"]\n",
                "[local-targets]\nproject-root = \"docs\"\n",
                "[html]\nstylesheet-files = [\"styles/manual.css\"]\n",
            ),
            Path::new("/workspace"),
        )
        .expect("portable relative paths");
        assert_eq!(
            config.resources.roots,
            [PathBuf::from("/workspace/docs/api")]
        );
        assert_eq!(
            config.local_targets.project_root,
            Some(PathBuf::from("/workspace/docs"))
        );
        assert_eq!(
            config.html.stylesheet_files,
            [PathBuf::from("/workspace/styles/manual.css")]
        );
    }

    #[test]
    fn errors_never_echo_attribute_values() {
        let error = ResolvedProjectConfig::parse(
            "schema-version = 2\n[analysis.attributes.secret]\nvalue = \"do-not-log\"\nunset = true",
            Path::new("/workspace"),
        )
        .expect_err("ambiguous attribute");
        assert!(!error.to_string().contains("do-not-log"));
    }

    #[test]
    fn discovery_stops_at_boundary_and_loads_content_addressed_snapshot() {
        let root = TestDirectory::new();
        let nested = root.0.join("docs/guide");
        fs::create_dir_all(&nested).expect("create nested directory");
        let config_path = root.0.join(FILE_NAME);
        fs::write(&config_path, "schema-version = 2\n").expect("write config");
        let input = nested.join("index.adoc");
        fs::write(&input, "= Guide\n").expect("write input");

        assert_eq!(
            discover(&input, &root.0).expect("discover config"),
            Some(config_path.canonicalize().expect("canonical config"))
        );
        let first = discover_and_load(&input, &root.0)
            .expect("load config")
            .expect("found config");
        let second = ConfigSnapshot::load(&config_path).expect("reload config");
        assert_eq!(first, second);
        assert_ne!(first.content_sha256, [0; 32]);
    }

    #[test]
    fn discovery_for_a_missing_document_starts_from_its_existing_parent() {
        let root = TestDirectory::new();
        fs::create_dir_all(root.0.join("docs/new")).expect("document parent");
        fs::write(root.0.join(FILE_NAME), "schema-version = 2\n").expect("project config");
        let policy = LocalTargetPolicy::new(&root.0).expect("boundary policy");

        let snapshot =
            discover_and_load_with_policy(&root.0.join("docs/new/unsaved.adoc"), &policy)
                .expect("configuration discovery")
                .expect("project configuration");

        assert_eq!(snapshot.path, root.0.join(FILE_NAME));
    }

    #[test]
    fn discovery_rejects_starts_outside_boundary() {
        let root = TestDirectory::new();
        let other = TestDirectory::new();
        assert_eq!(
            discover(&other.0, &root.0)
                .expect_err("outside boundary")
                .code,
            ConfigErrorCode::OutsideBoundary
        );
    }

    #[test]
    #[cfg(unix)]
    fn discovery_rejects_a_symbolic_linked_project_file() {
        use std::os::unix::fs::symlink;

        let project = TestDirectory::new();
        let outside = TestDirectory::new();
        fs::write(outside.0.join("config.toml"), "schema-version = 2\n").expect("outside config");
        symlink(outside.0.join("config.toml"), project.0.join(FILE_NAME)).expect("config symlink");

        let error = discover_and_load(&project.0, &project.0).expect_err("symlink rejected");
        assert_eq!(error.code, ConfigErrorCode::ReadFailed);
    }

    #[cfg(unix)]
    #[test]
    fn explicit_load_keeps_accepting_a_symbolic_linked_configuration() {
        use std::os::unix::fs::symlink;

        let project = TestDirectory::new();
        let outside = TestDirectory::new();
        let target = outside.0.join("config.toml");
        let selected = project.0.join("selected.toml");
        fs::write(&target, "schema-version = 2\n").expect("target configuration");
        symlink(&target, &selected).expect("selected configuration symlink");

        let snapshot = ConfigSnapshot::load(&selected).expect("explicit configuration");

        assert_eq!(
            snapshot.path,
            target.canonicalize().expect("canonical target")
        );
        assert_eq!(snapshot.config.schema_version, SCHEMA_VERSION);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn discovery_reads_from_the_boundary_handle_after_namespace_replacement() {
        use std::os::unix::fs::symlink;

        let project = TestDirectory::new();
        let outside = TestDirectory::new();
        fs::write(project.0.join(FILE_NAME), "schema-version = 2\n").expect("trusted config");
        fs::write(outside.0.join(FILE_NAME), "schema-version = 99\n").expect("outside config");
        let displaced = project.0.with_extension("anchored");

        let loaded = discover_loaded_with(&project.0, &project.0, || {
            fs::rename(&project.0, &displaced).expect("displace project");
            symlink(&outside.0, &project.0).expect("replace project path");
        });

        fs::remove_file(&project.0).expect("remove replacement symlink");
        fs::rename(&displaced, &project.0).expect("restore project");
        let loaded = loaded
            .expect("confined discovery")
            .expect("project configuration");
        let snapshot = ConfigSnapshot::from_loaded(loaded).expect("trusted configuration");
        assert_eq!(snapshot.config.schema_version, SCHEMA_VERSION);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn discovery_start_keeps_the_boundary_namespace_after_replacement() {
        use std::os::unix::fs::symlink;

        let project = TestDirectory::new();
        fs::create_dir_all(project.0.join("docs/sub")).expect("trusted start directory");
        fs::create_dir_all(project.0.join("docs/other")).expect("alternate trusted directory");
        fs::write(project.0.join("docs/sub/index.adoc"), "= Trusted\n").expect("trusted start");
        fs::write(
            project.0.join("docs/sub").join(FILE_NAME),
            "schema-version = 2\n",
        )
        .expect("trusted config");
        fs::write(project.0.join("docs/other/index.adoc"), "= Redirected\n")
            .expect("alternate start");
        fs::write(
            project.0.join("docs/other").join(FILE_NAME),
            "schema-version = 99\n",
        )
        .expect("redirected config");
        let policy = LocalTargetPolicy::new(&project.0).expect("boundary policy");
        let displaced = project.0.with_extension("anchored-start");

        fs::rename(&project.0, &displaced).expect("displace project");
        fs::create_dir_all(project.0.join("docs/other")).expect("replacement directory");
        fs::write(project.0.join("docs/other/index.adoc"), "= Replacement\n")
            .expect("replacement start");
        symlink("other", project.0.join("docs/sub")).expect("redirect replacement start");

        let snapshot =
            discover_and_load_with_policy(&project.0.join("docs/sub/index.adoc"), &policy)
                .expect("confined discovery")
                .expect("trusted configuration");

        assert_eq!(snapshot.config.schema_version, SCHEMA_VERSION);
        assert_eq!(snapshot.path, project.0.join("docs/sub").join(FILE_NAME));
        fs::remove_dir_all(&project.0).expect("remove replacement project");
        fs::rename(displaced, &project.0).expect("restore project");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preferred_policy_keeps_explicit_config_in_the_original_namespace() {
        use std::os::unix::fs::symlink;

        let project = TestDirectory::new();
        let outside = TestDirectory::new();
        let config_path = project.0.join(FILE_NAME);
        fs::write(&config_path, "schema-version = 2\n").expect("trusted config");
        fs::write(outside.0.join(FILE_NAME), "schema-version = 99\n").expect("outside config");
        let displaced = project.0.with_extension("anchored-explicit");
        let policy = LocalTargetPolicy::new(&project.0).expect("workspace policy");

        fs::rename(&project.0, &displaced).expect("displace project");
        symlink(&outside.0, &project.0).expect("replace project path");
        let loaded = ConfigSnapshot::load_with_preferred_policy(&config_path, &policy);

        fs::remove_file(&project.0).expect("remove replacement symlink");
        fs::rename(&displaced, &project.0).expect("restore project");
        let snapshot = loaded.expect("trusted configuration");
        assert_eq!(snapshot.config.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn analysis_snapshot_budget_is_transactional_at_each_boundary() {
        let limits = adocweave_workspace::RetainedResourceLimits {
            max_files: 2,
            max_total_bytes: 5,
            max_resource_bytes: 3,
        };
        let mut budget = AnalysisSnapshotBudget::new(limits);
        budget.charge(2).expect("first resource");
        assert_eq!((budget.resources(), budget.bytes()), (1, 2));
        assert_eq!(
            budget.charge(4),
            Err(AnalysisSnapshotLimitError::ResourceBytes)
        );
        assert_eq!((budget.resources(), budget.bytes()), (1, 2));
        budget.charge(3).expect("exact total");
        assert_eq!((budget.resources(), budget.bytes()), (2, 5));
        assert_eq!(
            budget.charge(0),
            Err(AnalysisSnapshotLimitError::ResourceCount)
        );
        assert_eq!((budget.resources(), budget.bytes()), (2, 5));
    }
}
