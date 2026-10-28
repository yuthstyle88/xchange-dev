//! DTOs of the `ledger_adjustments` view (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Read").

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The query key besides `branch`: `status` empty (every row) or one of `PENDING`, `APPROVED`, `REJECTED`,
/// `POSTED`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustments.ts")]
pub struct LedgerAdjustmentsFilter {
    #[serde(default)]
    pub status: String,
}

/// One line of an adjustment row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustments.ts")]
pub struct LedgerAdjustmentRowLine {
    pub currency: String,
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub cost_delta_thb: Option<String>,
}

/// Evidence attached to an adjustment row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustments.ts")]
pub struct LedgerAdjustmentRowEvidence {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub reference: String,
}

/// Valuation figures for a posted adjustment row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustments.ts")]
pub struct LedgerAdjustmentValuation {
    pub currency: String,
    pub quantity: String,
    pub valuation_thb: String,
}

/// One adjustment whose payload's branch is the view's branch. `status` is the request's, or `POSTED` for an
/// APPROVED one whose posting exists; `postingId`, `postedAt` and `valuations` are set only then. Times are
/// RFC 3339 UTC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustments.ts")]
pub struct LedgerAdjustmentRow {
    pub id: String,
    pub approval_request_id: String,
    pub status: String,
    pub kind: String,
    pub place: Option<String>,
    pub lines: Vec<LedgerAdjustmentRowLine>,
    pub reason: String,
    pub evidence: LedgerAdjustmentRowEvidence,
    pub exception_id: Option<String>,
    pub requested_by: String,
    pub requested_at: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision_reason: Option<String>,
    pub posting_id: Option<String>,
    pub posted_at: Option<String>,
    pub valuations: Vec<LedgerAdjustmentValuation>,
}

/// Newest request first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustments.ts")]
pub struct LedgerAdjustments {
    pub rows: Vec<LedgerAdjustmentRow>,
}
