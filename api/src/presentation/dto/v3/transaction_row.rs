//! Transaction row DTO for v3 (§7, §12.2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Shared transaction row in v3 admin views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transaction_row.ts")]
pub struct TransactionRow {
    pub id: String,
    pub date: String,
    pub created: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub principal: String,
    pub fee: String,
    pub bank_fee: String,
    pub settlement: String,
    pub method: String,
    pub customer: String,
    pub customer_id: String,
    pub staff: String,
    pub status: String,
    pub notes: Option<String>,
    pub cancel_reason: Option<String>,
    pub shift_id: Option<String>,
    pub reference_rate: Option<String>,
    pub reference_value: Option<String>,
    pub reference_spread: Option<String>,
    pub reference_percent: Option<String>,
    pub payment_ref: Option<String>,
    pub slip_url: Option<String>,
}
