//! DTOs of `ledgerAdjustmentPost` (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Post").

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_post.ts")]
pub struct LedgerAdjustmentPostInput {
    pub branch: String,
    #[serde(default)]
    pub id: String,
}

/// One posted figure: QUANTITY's signed quantity and the THB it was valued at (`"3250.00"`), or COST's
/// `"0"` and its signed delta. Decimal strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_post.ts")]
pub struct LedgerAdjustmentPostedLine {
    pub currency: String,
    pub quantity: String,
    pub valuation_thb: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_post.ts")]
pub struct LedgerAdjustmentPostOutput {
    pub id: String,
    pub posting_id: String,
    pub lines: Vec<LedgerAdjustmentPostedLine>,
}
