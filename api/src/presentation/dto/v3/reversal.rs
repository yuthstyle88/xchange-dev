//! RV-2 (#682): the answer of `POST /api/transactions/{id}/reversal-requests` (design `reversal.md` §4.2). A
//! command DTO: no TS is committed (finding RG7 on #682).

use serde::{Deserialize, Serialize};
use xc_exchange::domain::reversal::reversal::Reversal;

/// camelCase; money as a 2-place string, the business date as `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReversalDto {
    pub id: String,
    pub status: String,
    pub original_transaction_id: String,
    pub reason: String,
    pub amount_thb: String,
    pub business_date: String,
    pub approval_request_id: String,
}

impl ReversalDto {
    pub fn from_domain(r: &Reversal) -> Self {
        Self {
            id: r.id.0.to_string(),
            status: r.status.as_str().to_owned(),
            original_transaction_id: r.original_transaction_id.0.to_string(),
            reason: r.reason.clone(),
            amount_thb: r.amount_thb.to_string(),
            business_date: r.business_date.to_string(),
            approval_request_id: r.approval_request_id.to_string(),
        }
    }
}
