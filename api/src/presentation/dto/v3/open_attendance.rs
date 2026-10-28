//! Staff corrections §8 note 4, §10 (`docs/design/staff-corrections.md`): the row of view `open_attendance`.
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A row of view `open_attendance`: another employee's clock-in, still open, from an earlier day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "open_attendance.ts")]
pub struct OpenAttendanceRow {
    pub id: String,
    pub employee_id: String,
    pub name: String,
    pub clock_in: String,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    OpenAttendanceRow::export_all(&cfg)?;
    Ok(())
}
