use std::collections::BTreeMap;

use adocweave_core::SourceId;
use adocweave_core::preprocess::{ResourceDocument, ResourceSnapshot};

#[cfg_attr(
    feature = "ts-rs",
    derive(ts_rs::TS),
    ts(export, export_to = "protocol.d.mts")
)]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SafeMode {
    Unsafe,
    Server,
    Safe,
    #[default]
    Secure,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdocWeaveError {
    pub code: String,
    pub message: String,
}

pub(crate) fn resource_snapshot(documents: BTreeMap<String, String>) -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::default();
    for (target, source) in documents {
        snapshot.insert(
            target.clone(),
            ResourceDocument {
                source_id: SourceId::new(target),
                source: source.into(),
            },
        );
    }
    snapshot
}
