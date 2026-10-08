//! Device jobs view DTOs for v3 (§7.2).

use serde::Serialize;
use ts_rs::TS;

/// A row in the `device_jobs` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "device_jobs.ts")]
pub struct DeviceJobRow {
    pub id: String,
    pub kind: String,
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub reference: String,
    pub status: String,
    pub actor: String,
    pub created: String,
}
