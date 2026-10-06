//! Proposal command payload.

use crate::domain::custody::value_objects::count_sheet::CountSheet;
use crate::domain::custody::value_objects::request_kind::RequestKind;
use xc_kernel::money::{CurrencyCode, Money};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub request_id: String,
    pub payload_hash: String,
    pub kind: RequestKind,
    pub receiver: Option<String>,
    pub note: String,
    pub evidence_keys: Vec<String>,
    pub counted: CountSheet,
    pub safe_count: CountSheet,
    pub currency: Option<CurrencyCode>,
    pub amount: Option<Money>,
}
