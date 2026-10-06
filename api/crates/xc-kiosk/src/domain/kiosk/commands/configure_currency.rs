//! Switching a currency of the kiosk on or off, with its counter rate.

/// Everything is text as the screen sent it; the service judges it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureCurrency {
    pub code: String,
    pub enabled: bool,
    pub buy: String,
    pub sell: String,
    pub previous_updated: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
    /// The request id of the rate save; the caller draws it.
    pub request_id: String,
}
