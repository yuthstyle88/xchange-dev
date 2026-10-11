//! The cash commands' and views' wire types (CA-8 #624; design `cash-shift-transfer.md` §4.4, §4.7; the lane
//! controller's G1 ruling on #624). camelCase; `qty` and `qtyReceived` are decimal strings of a whole number ≥ 0;
//! money is a string (W7). The path's `{id}` is merged into the input as `id`, and `branch` is the optional body field
//! the command seam reads.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

/// One counted or moved line: `qty` is a whole number ≥ 0, sent as a string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CashLineInput {
    pub currency: String,
    #[ts(type = "number")]
    pub denomination_id: i64,
    pub qty: String,
}

/// One received line of a transfer: `qtyReceived` is a whole number ≥ 0, sent as a string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CashReceivedLineInput {
    pub currency: String,
    #[ts(type = "number")]
    pub denomination_id: i64,
    pub qty_received: String,
}

/// `countSubmit` `POST /api/cash-counts`. `shiftId` is required for a CLOSE count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CountSubmitInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub shift_id: Option<String>,
    pub lines: Vec<CashLineInput>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The input of a command on one record, `{id}` from the path: `countVerify`, `handoverAccept`, `handoverReject`,
/// `transferSubmit` and `transferDispatch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CashRecordInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub id: String,
}

/// `handoverStart` `POST /api/handovers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct HandoverStartInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub from_shift_id: String,
    pub receiver_id: String,
    pub lines: Vec<CashLineInput>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `transferCreate` `POST /api/cash-transfers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct TransferCreateInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub to_branch_id: Option<String>,
    #[serde(default)]
    pub from_shift_id: Option<String>,
    #[serde(default)]
    pub to_shift_id: Option<String>,
    pub lines: Vec<CashLineInput>,
    pub note: String,
}

/// `transferReceive` `POST /api/cash-transfers/{id}/receive`. The session branch is the destination (G9 ruling).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct TransferReceiveInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub id: String,
    pub lines: Vec<CashReceivedLineInput>,
}

/// `transferReconcile` `POST /api/cash-transfers/{id}/reconcile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct TransferReconcileInput {
    #[serde(default)]
    pub branch: Option<String>,
    pub id: String,
    pub note: String,
}

/// The answer of `countSubmit`: the count's id (a count has no status column, D:179).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CountSubmitOutput {
    pub id: String,
}

/// The answer of `countVerify` (design §4.9): `VERIFIED`, or `AWAITING_FINANCE` with the approval request it
/// recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CountVerifyOutput {
    pub id: String,
    /// `VERIFIED` | `AWAITING_FINANCE`.
    pub status: String,
    pub approval_request_id: Option<String>,
}

/// The answer of a hand-over or transfer command: the record's id and status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash.ts")]
pub struct CashRecordOutput {
    pub id: String,
    pub status: String,
}

/// The `cash_counts` filter (§4.7): the count's shift, and `WAITING` (`verified_by IS NULL`) or `VERIFIED` (Q-A).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "cash_counts.ts")]
pub struct CashCountsFilter {
    pub shift: Option<String>,
    pub status: Option<String>,
}

impl ViewFilter for CashCountsFilter {
    fn check(&self) -> Result<(), crate::presentation::http::v3::error::V3Error> {
        match self.status.as_deref() {
            None | Some("WAITING") | Some("VERIFIED") => Ok(()),
            Some(_) => Err(crate::presentation::http::v3::error::V3Error::new(
                "validationFailed",
                Some("status"),
                "status must be WAITING or VERIFIED",
                actix_web::http::StatusCode::BAD_REQUEST,
            )),
        }
    }
}

/// A row of `cash_counts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_counts.ts")]
pub struct CashCountRow {
    pub id: String,
    pub branch_id: String,
    pub shift_id: Option<String>,
    pub kind: String,
    pub counted_by: String,
    pub counted_at: String,
    pub verified_by: Option<String>,
    pub verified_at: Option<String>,
    pub note: Option<String>,
    pub totals: Vec<CashCountTotalRow>,
}

/// One currency total of a count: counted, expected and their difference, as strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_counts.ts")]
pub struct CashCountTotalRow {
    pub currency: String,
    pub counted: String,
    pub expected: Option<String>,
    pub difference: Option<String>,
}

/// The `cash_transfers` filter (§4.7): the transfer's status and kind, the table's values (D:21-23).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "cash_transfers.ts")]
pub struct CashTransfersFilter {
    pub status: Option<String>,
    pub kind: Option<String>,
}

impl ViewFilter for CashTransfersFilter {}

/// A row of `cash_transfers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_transfers.ts")]
pub struct CashTransferRow {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub from_branch_id: String,
    pub to_branch_id: Option<String>,
    pub from_shift_id: Option<String>,
    pub to_shift_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub note: Option<String>,
    pub lines: Vec<CashTransferLineRow>,
}

/// One line of a transfer: sent and received quantities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_transfers.ts")]
pub struct CashTransferLineRow {
    pub currency: String,
    #[ts(type = "number")]
    pub denomination_id: i64,
    pub qty_sent: String,
    pub qty_received: Option<String>,
}

/// A row of `handovers_pending`: a hand-over waiting for the caller to accept or reject it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "handovers_pending.ts")]
pub struct HandoverPendingRow {
    pub id: String,
    pub from_shift_id: String,
    pub count_id: String,
    pub receiver_id: String,
    pub status: String,
    pub created_at: String,
}

/// `my_shift`: the caller's own OPEN shift in the branch, or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_shift.ts")]
pub struct MyShift {
    pub id: String,
    pub status: String,
    pub opened_at: String,
    pub balances: Vec<MyShiftBalance>,
}

/// One currency of the caller's shift: opening and expected, as strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_shift.ts")]
pub struct MyShiftBalance {
    pub code: String,
    pub opening: String,
    pub expected: String,
}
