//! Counter rate entity.

use chrono::{DateTime, Utc};
use xc_kernel::money::{CurrencyCode, Rate};

#[derive(Debug, Clone, PartialEq)]
pub struct CounterRate {
    pub code: CurrencyCode,
    pub name: String,
    pub symbol: Option<String>,
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
    pub updated_at: Option<DateTime<Utc>>,
    pub request_id: Option<String>,
    pub partner_name: String,
    pub partner_cost: Option<Rate>,
    pub reference: Option<serde_json::Value>,
}
