//! Booking machine step outcomes and context types.

use chrono::{DateTime, Utc};
use xc_kernel::money::Rate;

use crate::domain::booking::entities::booking::Booking;
use crate::domain::booking::value_objects::booking_rules::BookingRules;
use crate::domain::booking::value_objects::booking_status::BookingStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmStep {
    Unchanged {},
    Confirm {
        rate: Rate,
        expires_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelStep {
    Unchanged {},
    Cancel {},
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedeemStep {
    Repeat {},
    Use {},
}

#[derive(Debug, Clone)]
pub struct CreateFacts {
    pub rules: BookingRules,
    pub open_shift: bool,
    pub counter: Option<Rate>,
    pub member_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBooking {
    pub status: BookingStatus,
    pub rate: Rate,
    pub expires_at: Option<DateTime<Utc>>,
    pub member_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRequest {
    pub booking: Booking,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveCustomer {
    pub id: String,
    pub name: String,
}
