//! `SourceChannel` (design §4.1, §4.2, §5): who created the booking and its start state.

use super::booking_status::BookingStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceChannel {
    App,
    Desk,
    Partner,
}

impl SourceChannel {
    /// B5, §4.2, Q-B5: APP -> CONFIRMED, DESK -> CONFIRMED, PARTNER -> PENDING_CUSTOMER_CONFIRMATION.
    pub fn start_status(&self) -> BookingStatus {
        match self {
            Self::App | Self::Desk => BookingStatus::Confirmed,
            Self::Partner => BookingStatus::PendingCustomerConfirmation,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::App => "APP",
            Self::Desk => "DESK",
            Self::Partner => "PARTNER",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "APP" => Some(Self::App),
            "DESK" => Some(Self::Desk),
            "PARTNER" => Some(Self::Partner),
            _ => None,
        }
    }
}
