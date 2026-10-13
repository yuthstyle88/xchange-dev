//! `BookingStatus` (design §4.1, §4.2, §5): the 8 booking states and their place-holding rule.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BookingStatus {
    PendingCustomerConfirmation,
    Confirmed,
    CheckedIn,
    InService,
    Completed,
    Cancelled,
    NoShow,
    Expired,
}

impl BookingStatus {
    /// B1, §4.1: Holding states are PENDING_CUSTOMER_CONFIRMATION, CONFIRMED, CHECKED_IN,
    /// IN_SERVICE and COMPLETED.
    pub fn holds_place(&self) -> bool {
        matches!(
            self,
            Self::PendingCustomerConfirmation
                | Self::Confirmed
                | Self::CheckedIn
                | Self::InService
                | Self::Completed
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PendingCustomerConfirmation => "PENDING_CUSTOMER_CONFIRMATION",
            Self::Confirmed => "CONFIRMED",
            Self::CheckedIn => "CHECKED_IN",
            Self::InService => "IN_SERVICE",
            Self::Completed => "COMPLETED",
            Self::Cancelled => "CANCELLED",
            Self::NoShow => "NO_SHOW",
            Self::Expired => "EXPIRED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "PENDING_CUSTOMER_CONFIRMATION" => Some(Self::PendingCustomerConfirmation),
            "CONFIRMED" => Some(Self::Confirmed),
            "CHECKED_IN" => Some(Self::CheckedIn),
            "IN_SERVICE" => Some(Self::InService),
            "COMPLETED" => Some(Self::Completed),
            "CANCELLED" => Some(Self::Cancelled),
            "NO_SHOW" => Some(Self::NoShow),
            "EXPIRED" => Some(Self::Expired),
            _ => None,
        }
    }
}
