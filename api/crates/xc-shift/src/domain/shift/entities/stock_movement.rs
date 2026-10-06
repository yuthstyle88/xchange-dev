//! Stock movement entity.

use chrono::{DateTime, Utc};
use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::shift::value_objects::stock_delta::Direction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockMovement {
    pub id: String,
    pub shift_id: String,
    pub at: DateTime<Utc>,
    pub by: String,
    pub currency: CurrencyCode,
    pub direction: Direction,
    pub amount: Money,
    pub reason: String,
    pub request_id: String,
}
