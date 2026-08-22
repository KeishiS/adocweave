pub const PROTOCOL_SCHEMA_VERSION: u16 = 15;

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WasmProductSet {
    pub syntax: bool,
    pub canonical_ast: bool,
    pub html: bool,
    pub attribute_occurrences: bool,
    pub attribute_queries: bool,
    pub resource_queries: bool,
    pub diagnostics: bool,
    pub symbols: bool,
    pub projection: bool,
}

impl Default for WasmProductSet {
    fn default() -> Self {
        Self {
            syntax: false,
            canonical_ast: false,
            html: true,
            attribute_occurrences: false,
            attribute_queries: false,
            resource_queries: true,
            diagnostics: true,
            symbols: false,
            projection: true,
        }
    }
}
