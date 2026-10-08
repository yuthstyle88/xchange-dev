//! Confirm booking command.

use chrono::{DateTime, Utc};
use xc_kernel::money::Rate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmBooking {
    pub id: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub expected_rate: Option<Rate>,
}
