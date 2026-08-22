#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmDocumentMode {
    #[default]
    Fragment,
    Complete,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmSyntaxMode {
    #[default]
    Permissive,
    Strict,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmUnknownRole {
    #[default]
    Silent,
    Diagnostic,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmUnknownSourceLanguage {
    #[default]
    PreserveSanitized,
    OmitClass,
    Diagnostic,
}

#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS), ts(export, export_to = "protocol.d.mts"))]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmUnresolvedReferencePresentation {
    #[default]
    Target,
    LabelOnly,
    Hidden,
}
