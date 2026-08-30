//! Strict, versioned project configuration owned by project processing.
//!
//! Parsing a project file never grants filesystem or network authority. The
//! resolved paths and limits remain inputs to a host policy that must restrict
//! them to an independently trusted workspace boundary.
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use adocweave::output::diagnostics::{LintConfig, RuleSettings, Severity, lint_rule};
use adocweave::output::formatter::{FormatConfig, NewlineStyle};
use adocweave::output::html::{HtmlDocumentMode, RenderPolicy};
use adocweave::preprocess::PreprocessOptions;
use adocweave::{AnalysisOptions, SyntaxMode};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod schema;

/// Conventional project configuration filename.
pub const FILE_NAME: &str = ".adocweave.toml";

/// Configuration schema version accepted by this package.
pub const SCHEMA_VERSION: u32 = 2;

/// Stable category for configuration failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigErrorCode {
    /// Configuration or search path could not be read safely.
    ReadFailed,
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

/// Parsed configuration content fixed by project processing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LoadedProjectConfig {
    pub(crate) path: PathBuf,
    pub(crate) content_sha256: [u8; 32],
    pub(crate) config: ProjectConfig,
}

impl LoadedProjectConfig {
    /// Parses text already read through the request's project authority.
    pub(crate) fn from_utf8_source(path: PathBuf, source: &str) -> Result<Self, ConfigError> {
        let directory = path.parent().ok_or_else(|| {
            ConfigError::new(
                ConfigErrorCode::ReadFailed,
                "the project file has no parent directory",
            )
        })?;
        let content_sha256 = Sha256::digest(source.as_bytes()).into();
        let config = ProjectConfig::parse(source, directory)?;
        Ok(Self {
            path,
            content_sha256,
            config,
        })
    }
}

/// Include policy and bounded local resource settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResourceSettings {
    /// Whether include preprocessing is enabled.
    pub(crate) include: bool,
    /// Configuration-relative roots proposed to the host policy.
    pub(crate) roots: Vec<PathBuf>,
    /// Resource limits no greater than built-in ceilings.
    pub(crate) limits: crate::ProjectResourceLimits,
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
            limits: crate::ProjectResourceLimits::default(),
        }
    }
}

/// Local target validation settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LocalTargetSettings {
    /// Whether local target validation is enabled.
    pub(crate) enabled: bool,
    /// Configuration-relative project root proposed to the host policy.
    pub(crate) project_root: Option<PathBuf>,
}

/// Complete-document rendering and stylesheet settings.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct HtmlSettings {
    /// Deterministic HTML rendering policy.
    pub(crate) policy: RenderPolicy,
    /// Configuration-relative stylesheet files.
    pub(crate) stylesheet_files: Vec<PathBuf>,
    /// Authored stylesheet URLs, subject to the active URL policy.
    pub(crate) stylesheet_urls: Vec<String>,
}

/// Fully typed schema-version-2 project configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfig {
    /// Parsed schema version.
    pub(crate) schema_version: u32,
    /// Core syntax, attribute, and diagnostic options.
    pub(crate) analysis: AnalysisOptions,
    /// Include preprocessing options sharing analysis attributes and expansion limits.
    pub(crate) preprocess: PreprocessOptions,
    /// Local resource settings.
    pub(crate) resources: ResourceSettings,
    /// Local target validation settings.
    pub(crate) local_targets: LocalTargetSettings,
    /// Formatter settings.
    pub(crate) format: FormatConfig,
    /// Whether `format.newline` was present in the project file.
    pub(crate) format_newline_explicit: bool,
    /// Whether `format.final-newline` was present in the project file.
    pub(crate) format_final_newline_explicit: bool,
    /// HTML and stylesheet settings.
    pub(crate) html: HtmlSettings,
}

impl Default for ProjectConfig {
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

impl ProjectConfig {
    /// Parses strict TOML and resolves relative paths against `directory`.
    pub(crate) fn parse(source: &str, directory: &Path) -> Result<Self, ConfigError> {
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
    fn resolve(self, directory: &Path) -> Result<ProjectConfig, ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::new(
                ConfigErrorCode::UnsupportedSchema,
                "only schema version 2 is supported",
            )
            .at("schema-version"));
        }

        let mut resolved = ProjectConfig::default();
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
        resolved.preprocess.max_total_bytes =
            u32::try_from(resolved.resources.limits.max_total_bytes).map_err(|_| {
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
        let ceiling = crate::ProjectResourceLimits::default();
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
        let limits = crate::ProjectResourceLimits {
            max_files,
            max_total_bytes,
            max_resource_bytes,
        };
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
            limits,
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
    #[test]
    fn strict_project_config_resolves_shared_consumer_options() {
        let config = ProjectConfig::parse(
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
            config.resources.limits,
            crate::ProjectResourceLimits {
                max_files: 20,
                max_total_bytes: 4096,
                max_resource_bytes: 2048,
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
            let config = ProjectConfig::parse(source, Path::new("/workspace")).expect("valid");
            assert!(config.resources.include, "{source}");
            assert!(config.preprocess.enable_includes, "{source}");
        }

        let default = ProjectConfig::default();
        assert!(default.resources.include);
        assert!(default.preprocess.enable_includes);

        let disabled = ProjectConfig::parse(
            "schema-version = 2\n[resources]\ninclude = false\n",
            Path::new("/workspace"),
        )
        .expect("valid");
        assert!(!disabled.resources.include);
        assert!(!disabled.preprocess.enable_includes);
    }

    #[test]
    fn html_roles_must_be_class_tokens() {
        let error = ProjectConfig::parse(
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
                ProjectConfig::parse(source, Path::new("/workspace"))
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
            let config =
                ProjectConfig::parse(&source, Path::new("/workspace")).unwrap_or_else(|error| {
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
            assert!(ProjectConfig::parse(source, Path::new("/workspace")).is_err());
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
            let error = ProjectConfig::parse(source, Path::new("/workspace"))
                .expect_err("platform-specific path must be rejected");
            assert_eq!(error.code, ConfigErrorCode::InvalidPath, "{source}");
        }

        let config = ProjectConfig::parse(
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
        let error = ProjectConfig::parse(
            "schema-version = 2\n[analysis.attributes.secret]\nvalue = \"do-not-log\"\nunset = true",
            Path::new("/workspace"),
        )
        .expect_err("ambiguous attribute");
        assert!(!error.to_string().contains("do-not-log"));
    }
}
