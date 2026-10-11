//! A reversal (design §3.1, §6): Blueprint §06 `REQUESTED → APPROVED → POSTED | REJECTED`, linked to the exchange it
//! reverses (§07). "Expired" is derived from the window, never stored (§4.1).

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

use crate::domain::transaction::transaction::TransactionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReversalId(pub Uuid);

/// §06's four stored states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReversalStatus {
    Requested,
    Approved,
    Posted,
    Rejected,
}

impl ReversalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReversalStatus::Requested => "REQUESTED",
            ReversalStatus::Approved => "APPROVED",
            ReversalStatus::Posted => "POSTED",
            ReversalStatus::Rejected => "REJECTED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "REQUESTED" => Some(ReversalStatus::Requested),
            "APPROVED" => Some(ReversalStatus::Approved),
            "POSTED" => Some(ReversalStatus::Posted),
            "REJECTED" => Some(ReversalStatus::Rejected),
            _ => None,
        }
    }
}

/// A stored reversal, as its request answers it and the repository reads it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reversal {
    pub id: ReversalId,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub original_transaction_id: TransactionId,
    pub status: ReversalStatus,
    pub reason: String,
    /// The original's settled THB, Σ `settle_thb` (V-1).
    pub amount_thb: Money,
    /// The original's completion date in the branch timezone (R-1).
    pub business_date: NaiveDate,
    /// AP-1's request. Set in the transaction that inserts the row (RG6), so every reversal read outside that
    /// transaction has one.
    pub approval_request_id: Uuid,
    pub requested_by: String,
    pub requested_at: DateTime<Utc>,
}

/// What a request inserts: a REQUESTED row with no approval request yet (RG6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReversal {
    pub id: ReversalId,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub original_transaction_id: TransactionId,
    pub reason: String,
    pub amount_thb: Money,
    pub business_date: NaiveDate,
    pub requested_by: String,
    pub requested_at: DateTime<Utc>,
}

/// What posting writes (RV-3, #684): the posting, the reversal document and the amounts it reversed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedReversalRow {
    pub posting_id: Uuid,
    pub receipt_id: Uuid,
    pub shift_id: Option<String>,
    pub posted_by: String,
    pub posted_at: DateTime<Utc>,
    pub realized_margin_reversed_thb: Option<rust_decimal::Decimal>,
    pub cost_difference_thb: Option<rust_decimal::Decimal>,
}
