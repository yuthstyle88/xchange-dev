//! EX-5 (#1075): DTOs for branch alert settings view and save command.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_alert_settings.ts")]
pub struct StockFloorDto {
    pub currency: String,
    pub min_quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_alert_settings.ts")]
pub struct BranchAlertSettingsData {
    pub branch: String,
    pub stale_data_minutes: Option<i32>,
    pub floors: Vec<StockFloorDto>,
    pub version: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_alert_settings.ts")]
pub struct BranchAlertSettingsSaveInput {
    pub branch: String,
    #[serde(default)]
    pub stale_data_minutes: Option<i32>,
    #[serde(default)]
    pub floors: Vec<StockFloorDto>,
    #[serde(default)]
    pub expected_version: Option<i32>,
}
