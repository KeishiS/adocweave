#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentMode {
    #[default]
    Fragment,
    Complete,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SyntaxMode {
    #[default]
    Permissive,
    Strict,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UnknownRole {
    #[default]
    Silent,
    Diagnostic,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UnknownSourceLanguage {
    #[default]
    PreserveSanitized,
    OmitClass,
    Diagnostic,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UnresolvedReferencePresentation {
    #[default]
    Target,
    LabelOnly,
    Hidden,
}
