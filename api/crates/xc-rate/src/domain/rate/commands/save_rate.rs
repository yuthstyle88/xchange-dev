//! Save rate command.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRate {
    pub request_id: String,
    pub code: String,
    pub buy: String,
    pub sell: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
    pub previous_updated: String,
}
