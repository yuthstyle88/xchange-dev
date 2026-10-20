//! Transaction detail DTO for v3 B04 transaction view (ticket #678, design §4.2).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TransactionFilter {
    pub id: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for TransactionFilter {}

pub type TransactionDetailFilter = TransactionFilter;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TxLineDto {
    pub line_no: i32,
    pub currency: String,
    pub side: String,
    pub quantity: String,
    pub unit_size: String,
    pub rate_snapshot: String,
    pub thb_amount: String,
    pub fee_thb: String,
    pub settle_thb: String,
}

pub type TransactionLineDto = TxLineDto;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TxPaymentDto {
    pub method: String,
    pub reference: String,
    pub account: String,
    pub slip_file_key: Option<String>,
    pub amount_thb: String,
    pub bank_fee_thb: String,
    pub paid_at: String,
}

pub type TransactionPaymentDto = TxPaymentDto;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TxReceiptDto {
    pub number: String,
    pub revision: i32,
    pub issued_at: String,
}

pub type ReceiptDto = TxReceiptDto;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TxApprovalDto {
    pub id: String,
    pub status: String,
}

pub type ApprovalRefDto = TxApprovalDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TxBenefitDto {
    pub rule_id: String,
    pub version_id: String,
    pub benefit_type: String,
    pub benefit_class: String,
    #[serde(default)]
    pub coupon_id: Option<String>,
    pub amount_thb: String,
    pub rate_before: String,
    pub rate_after: String,
    pub fee_before: String,
    pub fee_after: String,
    pub floor_capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TxBookingRefDto {
    pub id: String,
    pub booking_no: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TransactionDetailDto {
    pub id: String,
    pub status: String,
    pub document_ref: Option<String>,
    pub branch_id: String,
    pub customer_id: Option<String>,
    pub quote_id: Option<String>,
    pub quote_expires_at: Option<String>,
    pub lines: Vec<TxLineDto>,
    pub payment: Option<TxPaymentDto>,
    pub receipts: Vec<TxReceiptDto>,
    pub approval_request: Option<TxApprovalDto>,
    pub completed_at: Option<String>,
    pub cancelled_at: Option<String>,
    pub cancel_reason: Option<String>,
    pub reversal: Option<crate::presentation::dto::v3::reversal::ReversalSummaryDto>,
    pub benefits: Vec<TxBenefitDto>,
    pub booking: Option<TxBookingRefDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction.ts")]
pub struct TransactionViewData {
    pub transaction: Option<TransactionDetailDto>,
}

pub type TransactionDetailData = TransactionViewData;

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransactionFilter::export_all(&cfg)?;
    TxLineDto::export_all(&cfg)?;
    TxPaymentDto::export_all(&cfg)?;
    TxReceiptDto::export_all(&cfg)?;
    TxApprovalDto::export_all(&cfg)?;
    TxBenefitDto::export_all(&cfg)?;
    TxBookingRefDto::export_all(&cfg)?;
    TransactionDetailDto::export(&cfg)?;
    TransactionViewData::export(&cfg)?;
    Ok(())
}
