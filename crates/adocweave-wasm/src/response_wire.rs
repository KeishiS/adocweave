use crate::{WasmMathLanguage, WasmProductSet, WasmSeverity};

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmResponse {
    pub package_version: String,
    pub version: u32,
    pub generation: u32,
    pub products: WasmProductSet,
    pub parse: ParseSummary,
    pub syntax: String,
    pub ast: String,
    pub html: String,
    pub attribute_occurrences: Vec<WasmDocumentAttributeOccurrence>,
    pub attribute_queries: WasmAttributeQueryProduct,
    pub resource_queries: Vec<WasmResourceQuery>,
    pub diagnostics: Vec<WasmDiagnostic>,
    pub render_diagnostics: Vec<WasmDiagnostic>,
    pub symbols: Vec<WasmDocumentSymbol>,
    pub projection: Option<WasmDocumentProjection>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmApplicability {
    Always,
    Maybe,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmAttributeBindingQuery {
    pub id: u32,
    pub source_id: Option<String>,
    pub operation: WasmDocumentAttributeOperation,
    pub effective_value: Option<String>,
    pub error: Option<WasmAttributeExpansionError>,
    pub occurrence: WasmDocumentAttributeOccurrence,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmAttributeExpansionError {
    Undefined,
    Cycle,
    DepthLimitExceeded,
    SizeLimitExceeded,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmAttributeQueryProduct {
    pub bindings: Vec<WasmAttributeBindingQuery>,
    pub references: Vec<WasmAttributeReferenceQuery>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmAttributeReferenceQuery {
    pub source_id: Option<String>,
    pub range: WasmTextRange,
    pub name_range: WasmTextRange,
    pub name: String,
    pub binding_id: Option<u32>,
    pub effective_value: Option<String>,
    pub error: Option<WasmAttributeExpansionError>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmAttributeValueContinuation {
    Soft,
    Hard,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmBibliographyEntry {
    pub id: String,
    pub label: Option<String>,
    pub definition_range: WasmTextRange,
    pub references: Vec<WasmTextRange>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmBlockPresentationKind {
    Admonition,
    Quote,
    Verse,
    Example,
    Sidebar,
    Open,
    Collapsible,
    Figure,
    Table,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmBlockPresentationProjection {
    pub kind: WasmBlockPresentationKind,
    pub source_range: WasmTextRange,
    pub content_range: WasmTextRange,
    pub title: Option<String>,
    pub attribution: Option<String>,
    pub citation: Option<String>,
    pub roles: Vec<String>,
    pub open: Option<bool>,
    pub caption: Option<String>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmCitationAttributeProjection {
    pub source_range: WasmTextRange,
    pub name: Option<String>,
    pub value: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmCitationKeyProjection {
    pub source_range: WasmTextRange,
    pub key: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmCitationProjection {
    pub order: u32,
    pub source_range: WasmTextRange,
    pub keys: Vec<WasmCitationKeyProjection>,
    pub attributes: Vec<WasmCitationAttributeProjection>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDiagnostic {
    pub id: String,
    pub code: String,
    pub severity: WasmSeverity,
    pub message: String,
    pub range: WasmTextRange,
    pub related: Vec<WasmRelatedInformation>,
    pub fixes: Vec<WasmFix>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentAttributeContinuation {
    pub kind: WasmAttributeValueContinuation,
    pub range: WasmTextRange,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
/// One source-preserving standard document-attribute occurrence.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentAttributeOccurrence {
    pub range: WasmTextRange,
    pub name_range: WasmTextRange,
    pub name: String,
    pub value: WasmDocumentAttributeValue,
    pub operation: WasmDocumentAttributeOperation,
    pub valid: bool,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmDocumentAttributeOperation {
    Set,
    Unset,
    Counter,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentAttributeValue {
    pub source_range: WasmTextRange,
    pub source_text: String,
    pub folded_text: String,
    pub lines: Vec<WasmDocumentAttributeValueLine>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentAttributeValueLine {
    pub range: WasmTextRange,
    pub indent_range: WasmTextRange,
    pub content_range: WasmTextRange,
    pub ending_range: WasmTextRange,
    pub continuation: Option<WasmDocumentAttributeContinuation>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentCatalogs {
    pub footnotes: Vec<WasmFootnote>,
    pub bibliography: Vec<WasmBibliographyEntry>,
    pub index: Vec<WasmIndexEntry>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentProjection {
    pub source_id: Option<String>,
    pub source_blocks: Vec<WasmSourceBlockProjection>,
    pub formulas: Vec<WasmFormulaProjection>,
    pub citations: Vec<WasmCitationProjection>,
    pub block_presentations: Vec<WasmBlockPresentationProjection>,
    pub ordered_lists: Vec<WasmOrderedListProjection>,
    pub reference_edges: Vec<WasmReferenceEdge>,
    pub external_links: Vec<WasmExternalLink>,
    pub searchable_text: WasmSearchableText,
    pub structure: WasmDocumentStructure,
    pub catalogs: WasmDocumentCatalogs,
    pub targets: Vec<WasmReferenceTarget>,
    pub title: Option<WasmProjectedText>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentStructure {
    pub headings: Vec<WasmStructuredHeading>,
    pub toc: Vec<WasmTocEntry>,
    pub manpage: Option<WasmManpage>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmDocumentSymbol {
    pub name: String,
    pub kind: WasmSymbolKind,
    pub range: WasmTextRange,
    pub selection_range: WasmTextRange,
    pub children: Vec<WasmDocumentSymbol>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmExternalLink {
    pub source_range: WasmTextRange,
    pub target_range: WasmTextRange,
    pub target: String,
    pub label: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmFix {
    pub title: String,
    pub applicability: WasmApplicability,
    pub edits: Vec<WasmTextEdit>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmFootnote {
    pub number: u32,
    pub id: Option<String>,
    pub definition_range: WasmTextRange,
    pub content_range: WasmTextRange,
    pub text: String,
    pub occurrences: Vec<WasmTextRange>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmFormulaKind {
    Inline,
    Block,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmFormulaProjection {
    pub kind: WasmFormulaKind,
    pub language: WasmMathLanguage,
    pub source_range: WasmTextRange,
    pub content_range: WasmTextRange,
    pub source: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmIndexEntry {
    pub terms: Vec<String>,
    pub display: String,
    pub occurrences: Vec<WasmTextRange>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmMacroForm {
    Inline,
    Block,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmManpage {
    pub name: String,
    pub section: String,
    pub purpose: String,
    pub title_range: WasmTextRange,
    pub name_range: WasmTextRange,
    pub purpose_range: WasmTextRange,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmOrderedListProjection {
    pub source_range: WasmTextRange,
    pub start: Option<u32>,
    pub reversed: bool,
    pub style: WasmOrderedListStyle,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmOrderedListStyle {
    Arabic,
    Decimal,
    Loweralpha,
    Upperalpha,
    Lowerroman,
    Upperroman,
    Lowergreek,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParseSummary {
    pub block_count: u32,
    pub node_count: u32,
    pub reference_count: u32,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmProjectedReferenceFailureKind {
    MissingReferenceTarget,
    MissingReferenceAnchor,
    AmbiguousReferenceTarget,
    ReferenceOutsideRoot,
    ReferenceResolverFailure,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmProjectedReferenceNotice {
    ReferenceResolutionFallback,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
pub enum WasmProjectedResolutionOutcome {
    Failed {
        kind: WasmProjectedReferenceFailureKind,
    },
    Resolved {
        href: String,
        display_text: Option<String>,
        notices: Vec<WasmProjectedReferenceNotice>,
    },
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmProjectedText {
    pub source_range: WasmTextRange,
    pub text: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmReferenceEdge {
    pub source_id: Option<String>,
    pub source_range: WasmTextRange,
    pub target: WasmReferenceKey,
    pub resolution: Option<WasmProjectedResolutionOutcome>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
pub enum WasmReferenceKey {
    Document {
        document: String,
        anchor: Option<String>,
    },
    Local {
        anchor: String,
    },
    Scheme {
        scheme: String,
        locator: String,
        anchor: Option<String>,
    },
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmReferenceTarget {
    pub kind: WasmReferenceTargetKind,
    pub id: String,
    pub label: String,
    pub id_range: WasmTextRange,
    pub target_range: WasmTextRange,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmReferenceTargetKind {
    DocumentTitle,
    Part,
    Section,
    ExplicitAnchor,
    InlineAnchor,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmRelatedInformation {
    pub range: WasmTextRange,
    pub message: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmResourcePurpose {
    Image,
    Icon,
    Audio,
    Video,
    VideoPoster,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmResourceQuery {
    pub purpose: WasmResourcePurpose,
    pub form: WasmMacroForm,
    pub owner_range: WasmTextRange,
    pub range: WasmTextRange,
    pub target_range: WasmTextRange,
    pub target: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmSearchTextKind {
    Prose,
    Code,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmSearchTextSegment {
    pub kind: WasmSearchTextKind,
    pub source_range: WasmTextRange,
    pub text: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmSearchableText {
    pub text: String,
    pub segments: Vec<WasmSearchTextSegment>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmSectionKind {
    DocumentTitle,
    Part,
    Section,
    Appendix,
    Discrete,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmSourceBlockProjection {
    pub source_range: WasmTextRange,
    pub content_range: WasmTextRange,
    pub title: Option<WasmProjectedText>,
    pub language_range: Option<WasmTextRange>,
    pub language: Option<String>,
    pub line_numbers: bool,
    pub start_line: Option<u32>,
    pub source: String,
    pub caption: Option<String>,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmStructuredHeading {
    pub kind: WasmSectionKind,
    pub level: u32,
    pub id: String,
    pub id_range: WasmTextRange,
    pub title: String,
    pub range: WasmTextRange,
    pub title_range: WasmTextRange,
    pub number: Vec<u32>,
    pub toc_included: bool,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmSymbolKind {
    DocumentTitle,
    Part,
    Section,
    ListItem,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmTextEdit {
    pub range: WasmTextRange,
    pub replacement: String,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
/// A half-open UTF-8 byte range in the submitted source.
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmTextRange {
    pub start: u32,
    pub end: u32,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmTocEntry {
    pub id: String,
    pub title: String,
    pub level: u32,
    pub number: Vec<u32>,
    pub range: WasmTextRange,
    pub children: Vec<WasmTocEntry>,
}
