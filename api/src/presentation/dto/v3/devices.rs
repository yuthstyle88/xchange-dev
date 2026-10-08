//! Devices view DTOs for v3 (§7.1).

use serde::Serialize;
use ts_rs::TS;

use super::common::Money;

/// One branch in the devices overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "devices.ts")]
pub struct DeviceBranch {
    pub id: String,
    pub name: String,
}

/// View DTO for the `devices` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "devices.ts")]
pub struct DevicesOverview {
    pub branch_id: String,
    pub branches: Vec<DeviceBranch>,
    pub printer_online: bool,
    pub printer_kind: String,
    pub printer_model: String,
    pub has_drawer: Option<bool>,
    pub drawer_agent_active: bool,
    pub balance_thb: Money,
    pub balance_usd: Money,
    #[ts(type = "number")]
    pub completed_count: i64,
}
