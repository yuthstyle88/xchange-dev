//! Staff corrections (`docs/design/staff-corrections.md` §3, §10): a manager closes another employee's
//! forgotten clock-in (`attendanceCloseFor`; the view's row is in `open_attendance.rs`) and a verifier rejects a waiting count
//! (`countReject`).
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `attendanceCloseFor` `POST /api/attendance/{id}/close`: the row, the real clock-out time and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_corrections.ts")]
pub struct CloseForInput {
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    /// The attendance row (`attendance.id`), from the path.
    pub id: String,
    /// RFC 3339 with an offset.
    #[serde(default)]
    pub clock_out: String,
    #[serde(default)]
    pub reason: String,
}

/// The answer of `attendanceCloseFor`: the row as it now stands (RFC 3339, milliseconds, UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_corrections.ts")]
pub struct AttendanceClosed {
    pub id: String,
    pub employee_id: String,
    pub clock_in: String,
    pub clock_out: String,
}

/// `countReject` `POST /api/cash-counts/{id}/reject`: the count and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_corrections.ts")]
pub struct CountRejectInput {
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    pub id: String,
    #[serde(default)]
    pub reason: String,
}

/// The answer of `countReject`. `status` is always `REJECTED`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_corrections.ts")]
pub struct CountRejected {
    pub id: String,
    pub status: String,
    pub rejected_by: String,
    pub rejected_at: String,
    pub reason: String,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CloseForInput::export_all(&cfg)?;
    AttendanceClosed::export_all(&cfg)?;
    CountRejectInput::export_all(&cfg)?;
    CountRejected::export_all(&cfg)?;
    Ok(())
}
