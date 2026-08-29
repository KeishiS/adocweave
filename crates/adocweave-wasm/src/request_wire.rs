use std::collections::BTreeMap;

use serde::Deserialize;

use crate::object_deserialize::serde_object;
use crate::{
    DocumentMode, GeneratedBibliography, MathLanguage, ResolvedCitation, ResolvedReference,
    ResolvedResource, SafeMode, Severity, SyntaxMode, UnknownRole, UnknownSourceLanguage,
    UnresolvedReferencePresentation,
};

#[cfg(not(target_arch = "wasm32"))]
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[cfg(target_arch = "wasm32")]
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Present<T> {
        Value(T),
        Undefined(()),
    }

    Ok(match Present::deserialize(deserializer)? {
        Present::Value(value) => Some(value),
        Present::Undefined(()) => None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn requested<'de, D>(deserializer: D) -> Result<Option<()>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match bool::deserialize(deserializer)? {
        true => Ok(Some(())),
        false => Err(serde::de::Error::custom("product value must be true")),
    }
}

#[cfg(target_arch = "wasm32")]
fn requested<'de, D>(deserializer: D) -> Result<Option<()>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Selection {
        Enabled(bool),
        Undefined(()),
    }

    match Selection::deserialize(deserializer)? {
        Selection::Enabled(true) => Ok(Some(())),
        Selection::Enabled(false) => Err(serde::de::Error::custom("product value must be true")),
        Selection::Undefined(()) => Ok(None),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HtmlSelection {
    Enabled(bool),
    Options(Box<HtmlOptions>),
    #[cfg(target_arch = "wasm32")]
    Undefined(()),
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
        #[cfg(target_arch = "wasm32")]
        HtmlSelection::Undefined(()) => Ok(None),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DiagnosticSelection {
    Enabled(bool),
    Options(DiagnosticOptions),
    #[cfg(target_arch = "wasm32")]
    Undefined(()),
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
        #[cfg(target_arch = "wasm32")]
        DiagnosticSelection::Undefined(()) => Ok(None),
    }
}

serde_object! {
    #[cfg_attr(
        feature = "ts-rs",
        derive(ts_rs::TS),
        ts(export, export_to = "protocol.d.mts")
    )]
    #[derive(Clone, Debug, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct AnalyzeRequest as AnalyzeRequestObject {
        pub source: SourceInput,
        pub products: ProductRequest,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "ResourceInput"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub resources: Option<ResourceInput>,
    }
}

serde_object! {
    #[cfg_attr(
        feature = "ts-rs",
        derive(ts_rs::TS),
        ts(export, export_to = "protocol.d.mts")
    )]
    #[derive(Clone, Debug, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct SourceInput as SourceInputObject {
        pub text: String,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub id: Option<String>,
        #[cfg_attr(
            feature = "ts-rs",
            ts(optional, type = "{ [key in string]: string | null }")
        )]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub attributes: Option<BTreeMap<String, Option<String>>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "SyntaxMode"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub syntax_mode: Option<SyntaxMode>,
    }
}

serde_object! {
    #[cfg_attr(
        feature = "ts-rs",
        derive(ts_rs::TS),
        ts(export, export_to = "protocol.d.mts")
    )]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct ProductRequest as ProductRequestObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub syntax: Option<()>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub canonical_ast: Option<()>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true | HtmlOptions"))]
        #[wire_field(serde(default, deserialize_with = "html_product"))]
        pub html: Option<HtmlOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub attribute_occurrences: Option<()>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub attribute_queries: Option<()>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub resource_queries: Option<()>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true | DiagnosticOptions"))]
        #[wire_field(serde(default, deserialize_with = "diagnostic_product"))]
        pub diagnostics: Option<DiagnosticOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub symbols: Option<()>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "true"))]
        #[wire_field(serde(default, deserialize_with = "requested"))]
        pub document: Option<()>,
    }
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

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct DiagnosticOptions as DiagnosticOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "{ [key in string]: string | null }"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub protected_attributes: Option<BTreeMap<String, Option<String>>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "AuthoredUrlOptions"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub authored_urls: Option<AuthoredUrlOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "{ [key in string]: RuleOptions }"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub rules: Option<BTreeMap<String, RuleOptions>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "number"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub max_diagnostics: Option<u32>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct AuthoredUrlOptions as AuthoredUrlOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allowed_schemes: Option<Vec<String>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allow_relative: Option<bool>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct RuleOptions as RuleOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub enabled: Option<bool>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Severity"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub severity: Option<Severity>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct HtmlOptions as HtmlOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "DocumentMode"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub document_mode: Option<DocumentMode>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "ActiveUrlOptions"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub active_urls: Option<ActiveUrlOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "ExternalLinkOptions"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub external_links: Option<ExternalLinkOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "SourceLanguageOptions"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub source_languages: Option<SourceLanguageOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "RoleOptions"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub roles: Option<RoleOptions>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<MathLanguage>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub math_languages: Option<Vec<MathLanguage>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "UnresolvedReferencePresentation"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub unresolved_references: Option<UnresolvedReferencePresentation>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "ResourceCapabilities"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub resource_capabilities: Option<ResourceCapabilities>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<Stylesheet>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub stylesheets: Option<Vec<Stylesheet>>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct ActiveUrlOptions as ActiveUrlOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allowed_schemes: Option<Vec<String>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allow_authored_relative: Option<bool>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allow_resolved_relative: Option<bool>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allow_resolved_root_relative: Option<bool>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allow_data_uris: Option<bool>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct ExternalLinkOptions as ExternalLinkOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub open_in_new_context: Option<bool>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub noreferrer: Option<bool>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct SourceLanguageOptions as SourceLanguageOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allowed: Option<Vec<String>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "UnknownSourceLanguage"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub unknown: Option<UnknownSourceLanguage>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct RoleOptions as RoleOptionsObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allowed: Option<Vec<String>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "UnknownRole"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub unknown: Option<UnknownRole>,
    }
}

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct ResourceCapabilities as ResourceCapabilitiesObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub images: Option<bool>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "boolean"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub media: Option<bool>,
    }
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
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

serde_object! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    #[wire(default, rename_all = "camelCase", deny_unknown_fields)]
    pub struct ResourceInput as ResourceInputObject {
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "{ [key in string]: string }"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub documents: Option<BTreeMap<String, String>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub base_uri: Option<String>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "SafeMode"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub safe_mode: Option<SafeMode>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<string>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub allowed_schemes: Option<Vec<String>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "IncludeHandling"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub includes: Option<IncludeHandling>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ResolvedReference>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub references: Option<Vec<ResolvedReference>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ResolvedResource>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub assets: Option<Vec<ResolvedResource>>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ResolvedCitation>"))]
        #[wire_field(serde(default, deserialize_with = "present"))]
        pub citations: Option<Vec<ResolvedCitation>>,
        #[cfg_attr(feature = "ts-rs", ts(optional = nullable))]
        #[wire_field(serde(default))]
        pub bibliography: Option<GeneratedBibliography>,
    }
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
