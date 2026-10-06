//! The counter rate the kiosk asks the rate context to save.

use xc_kernel::money::CurrencyCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveCounterRate {
    pub code: CurrencyCode,
    pub buy: String,
    pub sell: String,
    pub previous_updated: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
    pub request_id: String,
}
