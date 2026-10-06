//! Custody request entity.

use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::count_sheet::CountSheet;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::request_kind::RequestKind;
use crate::domain::custody::value_objects::request_status::{
    Reconciliation, RequestStatus, TransferStage,
};
use crate::domain::custody::value_objects::signature::Signature;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::StockDelta;
use xc_kernel::money::{CurrencyCode, Money};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub by: String,
    pub at: CustodyTime,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustodyRequest {
    pub id: String,
    pub request_id: String,
    pub payload_hash: String,
    pub kind: RequestKind,
    pub status: RequestStatus,
    pub at: CustodyTime,
    pub sender: String,
    pub receiver: Option<String>,
    pub note: String,
    pub evidence_keys: Vec<String>,
    pub signatures: Vec<Signature>,
    pub shift_id: Option<String>,
    pub counted: Option<CountSheet>,
    pub target: Option<CashTarget>,
    pub target_difference: Option<StockDelta>,
    pub expected: Option<Balances>,
    pub variance: Option<StockDelta>,
    pub cutoff: Option<Cutoff>,
    pub safe_count: Option<CountSheet>,
    pub safe_expected: Option<Balances>,
    pub safe_variance: Option<StockDelta>,
    pub currency: Option<CurrencyCode>,
    pub amount: Option<Money>,
    pub stage: Option<TransferStage>,
    pub dispatched_at: Option<CustodyTime>,
    pub dispatched_by: Option<String>,
    pub rejected: Option<Decision>,
    pub completed_at: Option<CustodyTime>,
    pub reconciliation: Option<Reconciliation>,
    pub reconciled: Option<Decision>,
}
