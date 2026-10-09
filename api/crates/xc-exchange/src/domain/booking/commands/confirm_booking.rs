//! Confirm booking command.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmBooking {
    pub id: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub expected_rate: Option<Decimal>,
}
