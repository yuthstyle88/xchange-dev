//! `TransactionDto`: the answer of `POST /api/transactions`, `/{id}/confirm` and `/{id}/cancel`
//! (design `quote-transaction.md` §4.2). Its keys are `[required]` (lane controller ruling on #672,
//! issuecomment-6104120007): FE desk #539 consumes them. camelCase; money and quantities are
//! decimal strings (D-4), the line keys mirror `QuoteDto`'s.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_exchange::domain::transaction::transaction::ExchangeTransaction;
use xc_kernel::money::Side;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionLineDto {
    pub currency: String,
    pub shop_side: String,
    pub quantity: String,
    pub unit_size: String,
    pub rate_snapshot: String,
    pub thb_amount: String,
    pub fee_thb: String,
    pub settle_thb: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionPaymentDto {
    pub method: String,
    pub reference: String,
    pub account: String,
    pub amount_thb: String,
    pub bank_fee_thb: String,
    pub paid_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionDto {
    pub id: String,
    pub branch_id: String,
    pub mode: String,
    /// `DRAFT` | `PENDING_APPROVAL` | `CONFIRMED` | `COMPLETED` | `CANCELLED`.
    pub status: String,
    /// Set only when `COMPLETED` (D-3).
    pub document_ref: Option<String>,
    pub quote_id: Option<String>,
    pub customer_id: String,
    /// The approval request while (and after) the exchange waits for the branch manager.
    pub approval_request_id: Option<String>,
    pub cancel_reason: Option<String>,
    pub cancelled_at: Option<String>,
    pub recorded_at: String,
    pub lines: Vec<TransactionLineDto>,
    pub payment: TransactionPaymentDto,
}

impl TransactionDto {
    pub fn from_domain(t: &ExchangeTransaction) -> Self {
        Self {
            id: t.id.0.to_string(),
            branch_id: t.branch_id.as_str().to_owned(),
            mode: t.mode.as_str().to_owned(),
            status: t.status.as_str().to_owned(),
            document_ref: t.document_ref.clone(),
            quote_id: t.quote_id.map(|q| q.to_string()),
            customer_id: t.customer_id.clone(),
            approval_request_id: t.approval_request_id.map(|r| r.to_string()),
            cancel_reason: t.cancel_reason.clone(),
            cancelled_at: t.cancelled_at.map(|at| at.to_rfc3339()),
            recorded_at: t.recorded_at.to_rfc3339(),
            lines: t
                .lines
                .iter()
                .map(|l| TransactionLineDto {
                    currency: l.currency.to_string(),
                    shop_side: match l.shop_side {
                        Side::Buy => "BUY".to_owned(),
                        Side::Sell => "SELL".to_owned(),
                    },
                    quantity: l.quantity.to_string(),
                    unit_size: l.unit_size.to_string(),
                    rate_snapshot: l.rate_snapshot.to_string(),
                    thb_amount: l.thb_amount.to_string(),
                    fee_thb: l.fee_thb.to_string(),
                    settle_thb: l.settle_thb.to_string(),
                })
                .collect(),
            payment: TransactionPaymentDto {
                method: t.payment.method.as_str().to_owned(),
                reference: t.payment.reference.clone(),
                account: t.payment.account.clone(),
                amount_thb: t.payment.amount_thb.to_string(),
                bank_fee_thb: t.payment.bank_fee_thb.to_string(),
                paid_at: t.payment.paid_at.to_rfc3339(),
            },
        }
    }
}
