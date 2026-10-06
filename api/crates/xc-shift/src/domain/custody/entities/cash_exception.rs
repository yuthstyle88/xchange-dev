//! Cash exception entity for custody.

use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::shift::value_objects::stock_delta::StockDelta;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionStatus {
    Open,
    Reviewed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashException {
    pub id: String,
    pub request_id: String,
    pub status: ExceptionStatus,
    pub target_difference: Option<StockDelta>,
    pub cash_variance: Option<StockDelta>,
    pub safe_variance: Option<StockDelta>,
    pub note: String,
    pub at: CustodyTime,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<CustodyTime>,
}
