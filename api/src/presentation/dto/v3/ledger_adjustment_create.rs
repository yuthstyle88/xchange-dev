//! DTOs of `ledgerAdjustmentCreate` (design `docs/design/blueprint/ledger-inventory.md` §13.4, §13.5).
//! The body is the §13.4 payload as the wire carries it; amounts are decimal strings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One line: `quantity` on a QUANTITY adjustment, `costDeltaThb` on a COST one; both signed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_create.ts")]
pub struct LedgerAdjustmentLine {
    pub currency: String,
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub cost_delta_thb: Option<String>,
}

/// `files` are `files.key`s of the branch; `reference` is free text (a bank slip number).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_create.ts")]
pub struct LedgerAdjustmentEvidence {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub reference: String,
}

/// `kind` is `QUANTITY` or `COST`; `place` is QUANTITY's only, `SAFE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_create.ts")]
pub struct LedgerAdjustmentCreateInput {
    pub branch: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub lines: Vec<LedgerAdjustmentLine>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub evidence: LedgerAdjustmentEvidence,
    #[serde(default)]
    pub exception_id: Option<String>,
}

/// `status` is `REQUESTED`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "ledger_adjustment_create.ts")]
pub struct LedgerAdjustmentCreateOutput {
    pub id: String,
    pub approval_request_id: String,
    pub status: String,
}
