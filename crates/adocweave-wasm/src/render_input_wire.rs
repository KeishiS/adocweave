pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[cfg(not(target_arch = "wasm32"))]
fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[cfg(target_arch = "wasm32")]
fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Present<T> {
        Value(T),
        Undefined(()),
    }

    Ok(
        match <Present<T> as serde::Deserialize>::deserialize(deserializer)? {
            Present::Value(value) => Some(value),
            Present::Undefined(()) => None,
        },
    )
}

fn deserialize_optional_safe_integer<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = deserialize_present(deserializer)?;
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
        #[serde(
            default,
            deserialize_with = "deserialize_present",
            skip_serializing_if = "Option::is_none"
        )]
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
        display_text: Option<String>,
        #[serde(default)]
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<ReferenceNotice>"))]
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
        #[serde(
            default,
            deserialize_with = "deserialize_optional_safe_integer",
            skip_serializing_if = "Option::is_none"
        )]
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "number"))]
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
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<CitationSegment>"))]
        segments: Vec<CitationSegment>,
    },
    Failed {
        kind: ReferenceFailureKind,
    },
}

serde_object_serializable! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, serde::Serialize, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct CitationSegment as CitationSegmentObject {
        pub text: String,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
        #[wire_field(serde(default, deserialize_with = "deserialize_present", skip_serializing_if = "Option::is_none"))]
        pub anchor: Option<String>,
    }
}

serde_object_serializable! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, serde::Serialize, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct GeneratedBibliographyEntry as GeneratedBibliographyEntryObject {
        pub citation_key: String,
        pub text: String,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "string"))]
        #[wire_field(serde(default, deserialize_with = "deserialize_present", skip_serializing_if = "Option::is_none"))]
        pub label: Option<String>,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "number"))]
        #[wire_field(serde(default, deserialize_with = "deserialize_present", skip_serializing_if = "Option::is_none"))]
        pub number: Option<u32>,
    }
}

serde_object_serializable! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, serde::Serialize, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct GeneratedBibliography as GeneratedBibliographyObject {
        pub title: String,
        #[cfg_attr(feature = "ts-rs", ts(optional, type = "Array<GeneratedBibliographyEntry>"))]
        #[wire_field(serde(default))]
        pub entries: Vec<GeneratedBibliographyEntry>,
    }
}

serde_object_serializable! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, serde::Serialize, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct ResolvedCitation as ResolvedCitationObject {
        pub source_start: u32,
        pub source_end: u32,
        pub outcome: CitationOutcome,
    }
}

serde_object_serializable! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, serde::Serialize, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct ResolvedReference as ResolvedReferenceObject {
        pub source_start: u32,
        pub source_end: u32,
        pub outcome: ReferenceOutcome,
    }
}

serde_object_serializable! {
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
    #[derive(Clone, Debug, serde::Serialize, Eq, PartialEq)]
    #[wire(rename_all = "camelCase", deny_unknown_fields)]
    pub struct ResolvedResource as ResolvedResourceObject {
        pub source_start: u32,
        pub source_end: u32,
        pub outcome: ResourceOutcome,
    }
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
use crate::object_deserialize::serde_object_serializable;
