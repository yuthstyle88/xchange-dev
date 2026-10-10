//! `ServiceType` (design §5, Q2, owner Q-A1): WALK_IN and BOOKING, each with its own daily sequence.

/// The line a ticket waits in. Each type has its own sequence per branch and service date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceType {
    WalkIn,
    Booking,
}

impl ServiceType {
    /// The stored value: `"WALK_IN"` or `"BOOKING"` (`queue_tickets.service_type`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WalkIn => "WALK_IN",
            Self::Booking => "BOOKING",
        }
    }

    /// Parses the stored value; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "WALK_IN" => Some(Self::WalkIn),
            "BOOKING" => Some(Self::Booking),
            _ => None,
        }
    }

    /// The letter of the display number: `'W'` for WALK_IN, `'B'` for BOOKING (Q3).
    pub fn letter(&self) -> char {
        match self {
            Self::WalkIn => 'W',
            Self::Booking => 'B',
        }
    }
}
