//! Branch day command DTOs for v3 (CA-8b #1274).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DayStartServiceInput {
    pub branch: String,
    pub equipment_ready: bool,
    pub rates_verified: bool,
    pub rate_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DayStartServiceOutput {
    pub phase: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DayCutoffInput {
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DayCutoffOutput {
    pub phase: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DayTargetInput {
    pub branch: String,
    pub amounts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DayTargetOutput {
    pub success: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct AttendanceClockInput {
    pub branch: String,
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct AttendanceClockOutput {
    pub success: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct CustomerHoldCreateInput {
    pub branch: String,
    pub customer_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct CustomerHoldCreateOutput {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct CustomerHoldReleaseInput {
    pub branch: String,
    #[serde(default)]
    pub id: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct CustomerHoldReleaseOutput {
    pub success: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DraftBeginInput {
    pub branch: String,
    pub side: String,
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_day.ts")]
pub struct DraftBeginOutput {
    pub id: String,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DayStartServiceInput::export_all(&cfg)?;
    DayStartServiceOutput::export_all(&cfg)?;
    DayCutoffInput::export_all(&cfg)?;
    DayCutoffOutput::export_all(&cfg)?;
    DayTargetInput::export_all(&cfg)?;
    DayTargetOutput::export_all(&cfg)?;
    AttendanceClockInput::export_all(&cfg)?;
    AttendanceClockOutput::export_all(&cfg)?;
    CustomerHoldCreateInput::export_all(&cfg)?;
    CustomerHoldCreateOutput::export_all(&cfg)?;
    CustomerHoldReleaseInput::export_all(&cfg)?;
    CustomerHoldReleaseOutput::export_all(&cfg)?;
    DraftBeginInput::export_all(&cfg)?;
    DraftBeginOutput::export_all(&cfg)?;
    Ok(())
}
