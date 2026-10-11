//! Staff attendance view DTOs for v3 (§3.1).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_attendance.ts")]
pub struct StaffAttendanceRow {
    pub id: String,
    pub name: String,
    #[serde(rename = "in")]
    pub in_time: String,
    #[serde(rename = "out", skip_serializing_if = "Option::is_none")]
    pub out_time: Option<String>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StaffAttendanceRow::export_all(&cfg)?;
    Ok(())
}
