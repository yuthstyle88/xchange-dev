//! Admin cash view DTOs for v3 (§7, §12.1, §12.6, §12.9).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

/// Filter for the `admin_cash` view (`all=true` includes all 153 register currencies).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashFilter {
    pub all: bool,
}

impl ViewFilter for AdminCashFilter {}

/// Banner shift summary for the open shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashShift {
    pub id: String,
    pub opened_at: String,
    pub staff: String,
}

/// Stock row per currency shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashStockRow {
    pub code: String,
    pub opening: String,
    pub net: String,
    pub expected: String,
}

/// Cash flow stats in THB for the open shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashStats {
    pub opening: String,
    pub incoming: String,
    pub outgoing: String,
    pub remaining: String,
}

/// Reference spread summary over the open shift's exchanges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashReference {
    pub with_reference: u32,
    pub without_reference: u32,
    pub spread: String,
    pub percent: String,
}

/// Counted amount per currency from the last closed shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashCountedItem {
    pub code: String,
    pub amount: String,
}

/// Answer data for the `admin_cash` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashData {
    pub shift: Option<AdminCashShift>,
    pub stock: Vec<AdminCashStockRow>,
    pub stats: Option<AdminCashStats>,
    pub reference: AdminCashReference,
    pub currencies: Vec<String>,
    pub last_counted: Option<Vec<AdminCashCountedItem>>,
}
