//! Rate change entity.

use chrono::{DateTime, Utc};
use xc_kernel::money::{CurrencyCode, Rate};

#[derive(Debug, Clone, PartialEq)]
pub struct RateChange {
    pub code: CurrencyCode,
    pub buy: Rate,
    pub sell: Rate,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
    pub request_id: String,
    pub partner_name: String,
    pub partner_cost: Option<Rate>,
    pub reference: Option<serde_json::Value>,
}
