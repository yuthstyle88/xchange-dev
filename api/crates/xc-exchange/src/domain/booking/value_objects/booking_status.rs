//! Booking status value object.

/// A stored booking status.
///
/// EXPIRED is computed for display and is never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BookingStatus {
    Pending,
    Confirmed,
    Used,
    Cancelled,
}

impl BookingStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Confirmed => "CONFIRMED",
            Self::Used => "USED",
            Self::Cancelled => "CANCELLED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PENDING" => Some(Self::Pending),
            "CONFIRMED" => Some(Self::Confirmed),
            "USED" => Some(Self::Used),
            "CANCELLED" => Some(Self::Cancelled),
            _ => None,
        }
    }
}
