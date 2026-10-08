//! Device events view DTOs for v3 (§7.3).

use serde::Serialize;
use ts_rs::TS;

/// A row in the `device_events` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "device_events.ts")]
pub struct DeviceEventRow {
    pub seq: String,
    pub kind: String,
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub reference: String,
    pub at: String,
    pub actor: String,
    pub detail: String,
}
