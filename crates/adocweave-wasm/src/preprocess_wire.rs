use std::collections::{BTreeMap, BTreeSet};

use adocweave::SourceId;
use adocweave::preprocess::{PreprocessOptions, ResourceDocument, ResourceSnapshot, SafeMode};

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmSafeMode {
    Unsafe,
    Server,
    Safe,
    #[default]
    Secure,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmResource {
    pub source_id: String,
    pub source: String,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmPreprocessOptions {
    pub base_uri: Option<String>,
    pub safe_mode: WasmSafeMode,
    pub allowed_schemes: BTreeSet<String>,
    pub attributes: BTreeMap<String, Option<String>>,
    pub enable_includes: bool,
    pub max_include_depth: u32,
    pub max_includes: u32,
    pub max_total_bytes: u32,
    pub max_expanded_nodes: u32,
    pub max_source_map_segments: u32,
    pub max_attribute_expansion_depth: u32,
    pub max_attribute_expansion_bytes: u32,
}

impl Default for WasmPreprocessOptions {
    fn default() -> Self {
        Self {
            base_uri: None,
            safe_mode: WasmSafeMode::Secure,
            allowed_schemes: Default::default(),
            attributes: Default::default(),
            enable_includes: true,
            max_include_depth: 16,
            max_includes: 10000,
            max_total_bytes: 52428800,
            max_expanded_nodes: 1000000,
            max_source_map_segments: 1000000,
            max_attribute_expansion_depth: 32,
            max_attribute_expansion_bytes: 1048576,
        }
    }
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmAnalysisPreprocessInput {
    pub resources: BTreeMap<String, WasmResource>,
    pub options: WasmPreprocessOptions,
}

fn default_wasm_preprocess_request_source_id() -> Option<String> {
    None
}

fn default_wasm_preprocess_request_resources() -> BTreeMap<String, WasmResource> {
    Default::default()
}

fn default_wasm_preprocess_request_options() -> WasmPreprocessOptions {
    Default::default()
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmPreprocessRequest {
    pub package_version: String,
    #[serde(default = "default_wasm_preprocess_request_source_id")]
    pub source_id: Option<String>,
    pub source: String,
    #[serde(default = "default_wasm_preprocess_request_resources")]
    pub resources: BTreeMap<String, WasmResource>,
    #[serde(default = "default_wasm_preprocess_request_options")]
    pub options: WasmPreprocessOptions,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmPreprocessResponse {
    pub package_version: String,
    pub source: String,
    pub source_map: Vec<WasmSourceMapSegment>,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmSourceMapSegment {
    pub output_start: u32,
    pub output_end: u32,
    pub source_id: Option<String>,
    pub source_start: u32,
    pub source_end: u32,
    pub mapping: WasmSourceMapping,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WasmSourceMapping {
    Identity,
    WholeOrigin,
}

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmError {
    pub code: String,
    pub message: String,
}

pub(crate) fn resource_snapshot(resources: BTreeMap<String, WasmResource>) -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::default();
    for (target, resource) in resources {
        snapshot.insert(
            target,
            ResourceDocument {
                source_id: SourceId::new(resource.source_id),
                source: resource.source.into(),
            },
        );
    }
    snapshot
}

pub(crate) fn to_core_options(
    source_id: Option<SourceId>,
    options: WasmPreprocessOptions,
) -> PreprocessOptions {
    PreprocessOptions {
        source_id,
        base_uri: options.base_uri,
        safe_mode: match options.safe_mode {
            WasmSafeMode::Unsafe => SafeMode::Unsafe,
            WasmSafeMode::Server => SafeMode::Server,
            WasmSafeMode::Safe => SafeMode::Safe,
            WasmSafeMode::Secure => SafeMode::Secure,
        },
        allowed_schemes: options
            .allowed_schemes
            .into_iter()
            .map(|scheme| scheme.to_ascii_lowercase())
            .collect(),
        attributes: options.attributes,
        enable_includes: options.enable_includes,
        max_include_depth: options.max_include_depth,
        max_includes: options.max_includes,
        max_total_bytes: options.max_total_bytes,
        max_expanded_nodes: options.max_expanded_nodes,
        max_source_map_segments: options.max_source_map_segments,
        max_attribute_expansion_depth: options.max_attribute_expansion_depth,
        max_attribute_expansion_bytes: options.max_attribute_expansion_bytes,
    }
}
