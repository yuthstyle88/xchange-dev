//! Admin cash movements view DTOs for v3 (§7, §12.6).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A row in the `admin_cash_movements` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash_movements.ts")]
pub struct AdminCashMovementRow {
    pub id: String,
    pub created: String,
    pub shift_id: String,
    pub direction: String,
    pub currency: String,
    pub amount: String,
    pub reason: String,
    pub staff: String,
}
