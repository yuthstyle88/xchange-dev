//! Count line value object.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountLine {
    pub currency: String,
    pub denomination_id: i64,
    pub qty: i32,
}

impl CountLine {
    pub fn new(currency: &str, denomination_id: i64, qty: i32) -> Self {
        Self {
            currency: currency.to_string(),
            denomination_id,
            qty,
        }
    }
}
