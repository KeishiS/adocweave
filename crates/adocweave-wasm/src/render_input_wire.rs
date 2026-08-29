pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

fn deserialize_optional_safe_integer<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <Option<u64> as serde::Deserialize>::deserialize(deserializer)?;
    if value.is_some_and(|value| value > MAX_SAFE_INTEGER) {
        return Err(serde::de::Error::custom(
            "safe integer exceeds the JavaScript maximum",
        ));
    }
    Ok(value)
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ReferenceNotice {
    Fallback,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ReferenceFailureKind {
    MissingTarget,
    MissingAnchor,
    AmbiguousTarget,
    OutsideRoot,
    ResolverFailure,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceFailureKind {
    Missing,
    OutsideRoot,
    SchemeDenied,
    PermissionDenied,
    MediaTypeUnavailable,
    ResolverFailure,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
pub enum ReferenceOutcome {
    Resolved {
        href: String,
        #[serde(default)]
        display_text: Option<String>,
        #[serde(default)]
        notices: Vec<ReferenceNotice>,
    },
    Failed {
        kind: ReferenceFailureKind,
    },
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
pub enum ResourceOutcome {
    Resolved {
        href: String,
        media_type: String,
        #[serde(default, deserialize_with = "deserialize_optional_safe_integer")]
        byte_length: Option<u64>,
    },
    Failed {
        kind: ResourceFailureKind,
    },
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
pub enum CitationOutcome {
    Resolved {
        #[serde(default)]
        segments: Vec<CitationSegment>,
    },
    Failed {
        kind: ReferenceFailureKind,
    },
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CitationSegment {
    pub text: String,
    #[serde(default)]
    pub anchor: Option<String>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedBibliographyEntry {
    pub citation_key: String,
    pub text: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub number: Option<u32>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeneratedBibliography {
    pub title: String,
    #[serde(default)]
    pub entries: Vec<GeneratedBibliographyEntry>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedCitation {
    pub source_start: u32,
    pub source_end: u32,
    pub outcome: CitationOutcome,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedReference {
    pub source_start: u32,
    pub source_end: u32,
    pub outcome: ReferenceOutcome,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedResource {
    pub source_start: u32,
    pub source_end: u32,
    pub outcome: ResourceOutcome,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenderInputs {
    #[serde(default)]
    pub references: Vec<ResolvedReference>,
    #[serde(default)]
    pub resources: Vec<ResolvedResource>,
    #[serde(default)]
    pub citations: Vec<ResolvedCitation>,
    #[serde(default)]
    pub generated_bibliography: Option<GeneratedBibliography>,
}
