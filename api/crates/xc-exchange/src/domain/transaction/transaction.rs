//! ExchangeTransaction aggregate and related types.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use super::line::TransactionLine;
use super::payment::TransactionPayment;
use super::receipt_record::Receipt;
use xc_kernel::mode::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionStatus {
    Draft,
    PendingApproval,
    Confirmed,
    Completed,
    Cancelled,
}

impl TransactionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransactionStatus::Draft => "DRAFT",
            TransactionStatus::PendingApproval => "PENDING_APPROVAL",
            TransactionStatus::Confirmed => "CONFIRMED",
            TransactionStatus::Completed => "COMPLETED",
            TransactionStatus::Cancelled => "CANCELLED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "DRAFT" => Some(TransactionStatus::Draft),
            "PENDING_APPROVAL" => Some(TransactionStatus::PendingApproval),
            "CONFIRMED" => Some(TransactionStatus::Confirmed),
            "COMPLETED" => Some(TransactionStatus::Completed),
            "CANCELLED" => Some(TransactionStatus::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeTransaction {
    pub id: TransactionId,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub mode: Mode,
    pub status: TransactionStatus,
    pub document_ref: Option<String>,
    pub quote_id: Option<Uuid>,
    pub booking_id: Option<String>,
    pub shift_id: Option<String>,
    pub submission_id: String,
    pub payload_hash: String,
    pub customer_id: String,
    pub customer_name: String,
    pub nationality: String,
    pub staff_id: String,
    pub booking_no: String,
    pub notes: String,
    pub value_date: NaiveDate,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    pub approval_request_id: Option<Uuid>,
    pub passport_file_key: String,
    pub kiosk_request_id: Option<String>,
    pub staff_draft_id: Option<String>,
    pub count_confirmed: bool,
    pub lines: Vec<TransactionLine>,
    pub payment: TransactionPayment,
    pub receipts: Vec<Receipt>,
}
