//! Transaction draft entity for custody.

use crate::domain::custody::value_objects::custody_time::CustodyTime;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStatus {
    Open,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxDraft {
    pub id: String,
    pub shift_id: String,
    pub employee_id: String,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: ForeignAmount,
    pub at: CustodyTime,
    pub status: DraftStatus,
    pub document_ref: Option<String>,
}
