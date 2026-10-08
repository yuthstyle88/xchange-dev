//! Admin shift state view DTOs for v3 (§7, §12.2).

use serde::Serialize;
use ts_rs::TS;

/// Answer data for the `admin_shift_state` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_shift_state.ts")]
pub struct AdminShiftStateData {
    pub shift_open: bool,
    pub desk_url: String,
}
