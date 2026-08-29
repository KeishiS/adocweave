use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    DocumentMode, GeneratedBibliography, MathLanguage, ResolvedCitation, ResolvedReference,
    ResolvedResource, SafeMode, Severity, SyntaxMode, UnknownRole, UnknownSourceLanguage,
    UnresolvedReferencePresentation,
};

// `serde_wasm_bindgen` 0.6 deserializes a typed struct by requesting known
// properties and does not enumerate extra JavaScript properties for
// `deny_unknown_fields`. Flattening this rejecting map forces map traversal at
// the WASM boundary. `protocol-wasm.test.mjs` fixes that behavior with the
// generated module; JSON deserialization rejects the same fields.
type UnknownFields = BTreeMap<String, UnknownField>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnknownField;

impl<'de> Deserialize<'de> for UnknownField {
    fn deserialize<D>(_: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom("unknown field"))
    }
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn requested<'de, D>(deserializer: D) -> Result<Option<()>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match bool::deserialize(deserializer)? {
        true => Ok(Some(())),
        false => Err(serde::de::Error::custom("product value must be true")),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HtmlSelection {
    Enabled(bool),
    Options(Box<HtmlOptions>),
}

fn html_product<'de, D>(deserializer: D) -> Result<Option<HtmlOptions>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match HtmlSelection::deserialize(deserializer)? {
        HtmlSelection::Enabled(true) => Ok(Some(HtmlOptions::default())),
        HtmlSelection::Enabled(false) => {
            Err(serde::de::Error::custom("product value must be true"))
        }
        HtmlSelection::Options(options) => Ok(Some(*options)),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DiagnosticSelection {
    Enabled(bool),
    Options(DiagnosticOptions),
}

fn diagnostic_product<'de, D>(deserializer: D) -> Result<Option<DiagnosticOptions>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match DiagnosticSelection::deserialize(deserializer)? {
        DiagnosticSelection::Enabled(true) => Ok(Some(DiagnosticOptions::default())),
        DiagnosticSelection::Enabled(false) => {
            Err(serde::de::Error::custom("product value must be true"))
        }
        DiagnosticSelection::Options(options) => Ok(Some(options)),
    }
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeRequest {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    pub source: SourceInput,
    pub products: ProductRequest,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "ResourceInput"))]
    pub resources: Option<ResourceInput>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceInput {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    pub text: String,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(
        feature = "ts-rs",
        ts(optional, type = "{ [key in string]: string | null }")
    )]
    pub attributes: Option<BTreeMap<String, Option<String>>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "SyntaxMode"))]
    pub syntax_mode: Option<SyntaxMode>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProductRequest {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub syntax: Option<()>,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub canonical_ast: Option<()>,
    #[serde(default, deserialize_with = "html_product")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true | HtmlOptions"))]
    pub html: Option<HtmlOptions>,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub attribute_occurrences: Option<()>,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub attribute_queries: Option<()>,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub resource_queries: Option<()>,
    #[serde(default, deserialize_with = "diagnostic_product")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true | DiagnosticOptions"))]
    pub diagnostics: Option<DiagnosticOptions>,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub symbols: Option<()>,
    #[serde(default, deserialize_with = "requested")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
    pub document: Option<()>,
}

impl ProductRequest {
    pub(crate) fn is_empty(&self) -> bool {
        self.syntax.is_none()
            && self.canonical_ast.is_none()
            && self.html.is_none()
            && self.attribute_occurrences.is_none()
            && self.attribute_queries.is_none()
            && self.resource_queries.is_none()
            && self.diagnostics.is_none()
            && self.symbols.is_none()
            && self.document.is_none()
    }
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct DiagnosticOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(
        feature = "ts-rs",
        ts(optional, type = "{ [key in string]: string | null }")
    )]
    pub protected_attributes: Option<BTreeMap<String, Option<String>>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "AuthoredUrlOptions"))]
    pub authored_urls: Option<AuthoredUrlOptions>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(
        feature = "ts-rs",
        ts(optional, type = "{ [key in string]: RuleOptions }")
    )]
    pub rules: Option<BTreeMap<String, RuleOptions>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "number"))]
    pub max_diagnostics: Option<u32>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthoredUrlOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
    pub allowed_schemes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub allow_relative: Option<bool>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RuleOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Severity"))]
    pub severity: Option<Severity>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct HtmlOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "DocumentMode"))]
    pub document_mode: Option<DocumentMode>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "ActiveUrlOptions"))]
    pub active_urls: Option<ActiveUrlOptions>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "ExternalLinkOptions"))]
    pub external_links: Option<ExternalLinkOptions>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "SourceLanguageOptions"))]
    pub source_languages: Option<SourceLanguageOptions>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "RoleOptions"))]
    pub roles: Option<RoleOptions>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<MathLanguage>"))]
    pub math_languages: Option<Vec<MathLanguage>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(
        feature = "ts-rs",
        ts(optional, type = "UnresolvedReferencePresentation")
    )]
    pub unresolved_references: Option<UnresolvedReferencePresentation>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "ResourceCapabilities"))]
    pub resource_capabilities: Option<ResourceCapabilities>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<Stylesheet>"))]
    pub stylesheets: Option<Vec<Stylesheet>>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ActiveUrlOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
    pub allowed_schemes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub allow_authored_relative: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub allow_resolved_relative: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub allow_resolved_root_relative: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub allow_data_uris: Option<bool>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ExternalLinkOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub open_in_new_context: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub noreferrer: Option<bool>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SourceLanguageOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
    pub allowed: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "UnknownSourceLanguage"))]
    pub unknown: Option<UnknownSourceLanguage>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RoleOptions {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
    pub allowed: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "UnknownRole"))]
    pub unknown: Option<UnknownRole>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceCapabilities {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub images: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
    pub media: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
pub enum Stylesheet {
    External { url: String },
    Inline { css: String },
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ResourceInput {
    #[serde(flatten)]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    unknown_fields: UnknownFields,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "{ [key in string]: string }"))]
    pub documents: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
    pub base_uri: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "SafeMode"))]
    pub safe_mode: Option<SafeMode>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
    pub allowed_schemes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "IncludeHandling"))]
    pub includes: Option<IncludeHandling>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ResolvedReference>"))]
    pub references: Option<Vec<ResolvedReference>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ResolvedResource>"))]
    pub assets: Option<Vec<ResolvedResource>>,
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ResolvedCitation>"))]
    pub citations: Option<Vec<ResolvedCitation>>,
    #[cfg_attr(feature = "ts-rs", ts(optional = nullable))]
    pub bibliography: Option<GeneratedBibliography>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum IncludeHandling {
    #[default]
    Expand,
    Preserve,
}
