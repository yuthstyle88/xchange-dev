//! Staff floor view DTOs for v3 (§3.1).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_floor.ts")]
pub struct StaffFloorHours {
    pub open: String,
    pub close: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_floor.ts")]
pub struct StaffFloorMyShift {
    pub id: String,
    pub opened_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_floor.ts")]
pub struct StaffFloorCutoff {
    pub at: String,
    pub pending_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_floor.ts")]
pub struct StaffFloorTarget {
    pub amounts: HashMap<String, String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_floor.ts")]
pub struct StaffFloorView {
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours: Option<StaffFloorHours>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_clock_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub my_shift: Option<StaffFloorMyShift>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cutoff: Option<StaffFloorCutoff>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<StaffFloorTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe: Option<HashMap<String, String>>,
}

pub type StaffFloor = StaffFloorView;

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StaffFloorHours::export_all(&cfg)?;
    StaffFloorMyShift::export_all(&cfg)?;
    StaffFloorCutoff::export_all(&cfg)?;
    StaffFloorTarget::export_all(&cfg)?;
    StaffFloorView::export_all(&cfg)?;
    Ok(())
}
