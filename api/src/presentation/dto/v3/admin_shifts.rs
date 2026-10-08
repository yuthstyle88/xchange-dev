//! Admin shifts view DTOs for v3 (§7, §12.6).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A non-zero difference item (`counted - expected`) for a closed shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_shifts.ts")]
pub struct AdminShiftDifferenceItem {
    pub code: String,
    pub amount: String,
}

/// A currency balance detail row for a shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_shifts.ts")]
pub struct AdminShiftBalanceRow {
    pub code: String,
    pub expected: String,
    pub counted: Option<String>,
    pub difference: Option<String>,
}

/// A row in the `admin_shifts` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_shifts.ts")]
pub struct AdminShiftRow {
    pub id: String,
    pub status: String,
    pub opened_at: String,
    pub staff: String,
    pub note: String,
    pub difference: Option<Vec<AdminShiftDifferenceItem>>,
    pub balances: Vec<AdminShiftBalanceRow>,
}
