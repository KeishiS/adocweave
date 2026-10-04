//! Versioned, finite JSON boundary for the optional slides helper.

use super::{HostError, HostResult};
use adocweave_core::output::html::RichInline;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const INPUT_BYTES: usize = 4 * 1024 * 1024;
pub const OUTPUT_BYTES: usize = 32 * 1024 * 1024;
pub const ITEMS: usize = 1024;
pub const BIBLIOGRAPHY_ITEMS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Body,
    Notes,
}
impl Scope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Notes => "notes",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scopes<T> {
    pub body: T,
    pub notes: T,
}
impl<T> Scopes<T> {
    pub fn get(&self, scope: Scope) -> &T {
        match scope {
            Scope::Body => &self.body,
            Scope::Notes => &self.notes,
        }
    }
    pub fn iter(&self) -> [(Scope, &T); 2] {
        [(Scope::Body, &self.body), (Scope::Notes, &self.notes)]
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Eqnums {
    #[default]
    None,
    Ams,
    All,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Equation {
    pub key: String,
    pub tex: String,
    pub display: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CitationItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_author: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_only: Option<bool>,
}
impl CitationItem {
    pub fn new(id: String) -> Self {
        Self {
            id,
            locator: None,
            label: None,
            prefix: None,
            suffix: None,
            suppress_author: None,
            author_only: None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub key: String,
    pub items: Vec<CitationItem>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeInput {
    pub equations: Vec<Equation>,
    pub citations: Vec<Citation>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Macro {
    pub name: String,
    pub definition: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Csl {
    pub items: Vec<serde_json::Value>,
    pub style: String,
    pub locale: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Request {
    pub schema_version: u8,
    pub eqnums: Eqnums,
    pub scopes: Scopes<ScopeInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macros: Option<Vec<Macro>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csl: Option<Csl>,
}
impl Request {
    pub fn is_empty(&self) -> bool {
        self.scopes
            .iter()
            .iter()
            .all(|(_, s)| s.equations.is_empty() && s.citations.is_empty())
    }
    pub fn validate(&self) -> HostResult<()> {
        check(
            self.schema_version == 1,
            "unsupported request schemaVersion",
        )?;
        let mut equations = 0;
        let mut citations = 0;
        for (_, input) in self.scopes.iter() {
            let mut keys = BTreeSet::new();
            for equation in &input.equations {
                check(
                    valid_key(&equation.key) && keys.insert(&equation.key),
                    "invalid or duplicate request key",
                )?;
                check(
                    equation.tex.len() <= 16 * 1024,
                    "equation byte limit exceeded",
                )?;
                equations += 1;
            }
            for citation in &input.citations {
                check(
                    valid_key(&citation.key) && keys.insert(&citation.key),
                    "invalid or duplicate request key",
                )?;
                check(
                    !citation.items.is_empty() && citation.items.len() <= 64,
                    "citation item count limit exceeded",
                )?;
                for item in &citation.items {
                    check(
                        !item.id.is_empty() && item.id.len() <= 4096,
                        "invalid citation item id",
                    )?;
                    check(
                        !item.suppress_author.unwrap_or(false)
                            || !item.author_only.unwrap_or(false),
                        "citation author flags conflict",
                    )?;
                    check(
                        [&item.locator, &item.label, &item.prefix, &item.suffix]
                            .iter()
                            .all(|v| v.as_ref().is_none_or(|v| v.len() <= 4096)),
                        "citation item byte limit exceeded",
                    )?;
                }
                citations += 1;
            }
        }
        check(
            equations <= ITEMS && citations <= ITEMS,
            "request item count limit exceeded",
        )?;
        check(
            citations == 0 || self.csl.is_some(),
            "citations require CSL inputs",
        )?;
        if let Some(macros) = &self.macros {
            check(macros.len() <= 64, "macro count limit exceeded")?;
            let mut names = BTreeSet::new();
            for value in macros {
                check(
                    !value.name.is_empty()
                        && value.name.len() <= 64
                        && value.name.bytes().all(|b| b.is_ascii_alphabetic())
                        && !matches!(value.name.as_str(), "constructor" | "prototype")
                        && names.insert(&value.name),
                    "invalid or duplicate macro name",
                )?;
                check(
                    value.definition.len() <= 4096
                        && value.arguments.unwrap_or(0) <= 9
                        && value
                            .default
                            .as_ref()
                            .is_none_or(|v| v.len() <= 4096 && value.arguments.unwrap_or(0) > 0),
                    "invalid macro definition",
                )?;
            }
        }
        if let Some(csl) = &self.csl {
            check(
                csl.items.len() <= BIBLIOGRAPHY_ITEMS
                    && csl.style.len() <= 512 * 1024
                    && csl.locale.len() <= 512 * 1024,
                "CSL input limit exceeded",
            )?;
            let mut ids = BTreeSet::new();
            for item in &csl.items {
                let id = item
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| HostError::protocol("CSL item requires a string id"))?;
                check(
                    !id.is_empty() && id.len() <= 4096 && ids.insert(id),
                    "invalid or duplicate CSL item id",
                )?;
                validate_json(item, 0)?;
            }
        }
        Ok(())
    }
}
fn validate_json(value: &serde_json::Value, depth: usize) -> HostResult<()> {
    check(depth <= 32, "CSL JSON depth limit exceeded")?;
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                validate_json(value, depth + 1)?;
            }
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                check(
                    !matches!(key.as_str(), "__proto__" | "prototype" | "constructor"),
                    "forbidden CSL JSON key",
                )?;
                validate_json(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn valid_key(value: &str) -> bool {
    value.len() <= 64
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub fn check(condition: bool, message: &str) -> HostResult<()> {
    if condition {
        Ok(())
    } else {
        Err(HostError::protocol(message))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
pub enum EquationResult {
    Ok {
        key: String,
        svg: String,
        mathml: String,
    },
    Failed {
        key: String,
    },
}
impl EquationResult {
    pub fn key(&self) -> &str {
        match self {
            Self::Ok { key, .. } | Self::Failed { key } => key,
        }
    }
    pub fn failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "lowercase", deny_unknown_fields)]
pub enum CitationResult {
    Ok {
        key: String,
        inlines: Vec<RichInline>,
    },
    Failed {
        key: String,
    },
}
impl CitationResult {
    pub fn key(&self) -> &str {
        match self {
            Self::Ok { key, .. } | Self::Failed { key } => key,
        }
    }
    pub fn failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BibliographyEntry {
    pub id: String,
    pub inlines: Vec<RichInline>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeOutput {
    pub equations: Vec<EquationResult>,
    pub citations: Vec<CitationResult>,
    pub bibliography: Vec<BibliographyEntry>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    #[serde(deserialize_with = "nullable")]
    pub scope: Option<Scope>,
    #[serde(deserialize_with = "nullable")]
    pub key: Option<String>,
    pub severity: Severity,
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Response {
    pub schema_version: u8,
    pub scopes: Scopes<ScopeOutput>,
    pub diagnostics: Vec<Diagnostic>,
    pub notices: Notices,
}

// Nullable fields in diagnostics are required; absence is a protocol error.
fn nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MathNotices {
    pub font_attribution: String,
    pub font_license: String,
    pub lppl_license: String,
    pub mathjax_license: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CitationNotices {
    pub attribution: String,
    pub license: String,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Notices {
    #[serde(deserialize_with = "nullable")]
    pub math: Option<MathNotices>,
    #[serde(deserialize_with = "nullable")]
    pub citations: Option<CitationNotices>,
}
impl Notices {
    pub fn validate(&self, request: &Request, fatal: bool) -> HostResult<()> {
        let math = !fatal
            && request
                .scopes
                .iter()
                .iter()
                .any(|(_, scope)| !scope.equations.is_empty());
        let citations = !fatal
            && request
                .scopes
                .iter()
                .iter()
                .any(|(_, scope)| !scope.citations.is_empty());
        check(
            self.math.is_some() == math && self.citations.is_some() == citations,
            "helper notices do not match the included dependencies",
        )?;
        let mut total = 0;
        let mut text = |value: &str| -> HostResult<()> {
            check(
                !value.trim().is_empty()
                    && value.len() <= 64 * 1024
                    && !value
                        .chars()
                        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')),
                "invalid or oversized helper notice text",
            )?;
            total += value.len();
            check(
                total <= 256 * 1024,
                "helper notice total byte limit exceeded",
            )
        };
        if let Some(math) = &self.math {
            for value in [
                &math.font_attribution,
                &math.font_license,
                &math.lppl_license,
                &math.mathjax_license,
            ] {
                text(value)?;
            }
        }
        if let Some(citations) = &self.citations {
            text(&citations.attribution)?;
            text(&citations.license)?;
        }
        Ok(())
    }
}
