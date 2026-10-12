//! FN-1 #1068: F01's sheet and certification, `GET /api/v3/views/finance_reconciliation` and
//! `POST /api/v3/commands/reconciliation_certify` (design `executive-finance.md` §5.1, §8.2), camelCase. Money is a
//! decimal string with two places (W7). This file fixes the JSON shape of every part once, including the four FN-2
//! fills, so FN-2's `SheetPartsPort` answers these types.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One part of the sheet. An unavailable part names, in `reason`, the ticket it waits for, and has no `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct Part<T: TS> {
    pub available: bool,
    pub reason: Option<String>,
    pub data: Option<T>,
}

impl<T: TS> Part<T> {
    pub fn of(data: T) -> Self {
        Self {
            available: true,
            reason: None,
            data: Some(data),
        }
    }

    pub fn waiting_for(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
            data: None,
        }
    }
}

/// The branch-day's completed exchange totals per payment method, and the reversals posted that day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct TransactionTotals {
    pub by_method: Vec<MethodTotals>,
    pub reversed_thb: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct MethodTotals {
    pub method: String,
    pub buy_thb: String,
    pub sell_thb: String,
    pub fees: String,
    pub bank_fees: String,
    /// The COMPLETED transactions whose first payment is this method (ruling Q1).
    #[ts(type = "number")]
    pub count: i64,
}

/// One count of the day (filled by FN-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct CountRow {
    pub count_id: String,
    pub kind: String,
    pub counted_by: String,
    pub verified_by: Option<String>,
    pub lines: Vec<CountLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct CountLine {
    pub currency: String,
    pub counted: String,
    pub expected: String,
    pub difference: String,
}

/// The day's transfers (filled by FN-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct TransferSummary {
    pub by_status: Vec<TransferStatusCount>,
    pub in_transit: Vec<InTransitRow>,
    pub short_receipts: Vec<ShortReceiptRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct TransferStatusCount {
    pub status: String,
    #[ts(type = "number")]
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct InTransitRow {
    pub transfer_id: String,
    pub currency: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct ShortReceiptRow {
    pub transfer_id: String,
    pub currency: String,
    pub sent: String,
    pub received: String,
}

/// One open `cash_exceptions` row of the branch (ruling Q2: open rows only, so `reviewed` is `false`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct ExceptionRow {
    pub id: String,
    pub position: i32,
    pub reviewed: bool,
    pub summary: String,
}

/// One LG-7 reconcile check, counted (filled by FN-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct IntegrityRow {
    pub check: u8,
    pub ok: bool,
    pub offenders: u32,
}

/// One reversal request still waiting (filled by FN-2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct ReversalPendingRow {
    pub request_id: String,
    pub transaction_id: String,
    pub requested_by: String,
    pub created_at: String,
}

/// One certification of the branch-day; `stale` when its hash is not the sheet's current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct CertificationRow {
    pub id: String,
    pub certified_by: String,
    pub certified_at: String,
    pub note: Option<String>,
    pub figures_hash: String,
    pub stale: bool,
}

/// F01: the branch-day's six parts, the hash of its figures, and its certifications, latest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct FinanceSheet {
    pub branch: String,
    pub date: String,
    pub transactions: Part<TransactionTotals>,
    pub counts: Part<Vec<CountRow>>,
    pub transfers: Part<TransferSummary>,
    pub exceptions: Part<Vec<ExceptionRow>>,
    pub integrity: Part<Vec<IntegrityRow>>,
    pub reversals_pending: Part<Vec<ReversalPendingRow>>,
    pub figures_hash: String,
    pub certifications: Vec<CertificationRow>,
}

/// The view's query key besides `branch`: the business date, `YYYY-MM-DD`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct FinanceReconciliationFilter {
    #[serde(default)]
    pub date: String,
}

/// The certify command's body: the sheet's branch, day and hash as the caller read them, and a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "finance_reconciliation.ts")]
pub struct ReconciliationCertifyInput {
    pub branch: String,
    pub date: String,
    pub figures_hash: String,
    #[serde(default)]
    pub note: String,
}
