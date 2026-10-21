//! `Counter` (design §5, Q7, owner Q-A4): a teller with an OPEN shift in the branch, ACTIVE or
//! PAUSED. One row of `queue_counters`; it exists while the shift is open.

use xc_kernel::ids::BranchId;

use crate::domain::ticket::queue_error::QueueError;

/// `queue_counters.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CounterStatus {
    Active,
    Paused,
}

impl CounterStatus {
    /// The stored value: `ACTIVE` or `PAUSED`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
        }
    }

    /// Parses the stored value; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ACTIVE" => Some(Self::Active),
            "PAUSED" => Some(Self::Paused),
            _ => None,
        }
    }

    /// A PAUSED counter cannot call (`QueueError::CounterPaused`, §4.3).
    pub fn can_call(&self) -> Result<(), QueueError> {
        match self {
            Self::Active => Ok(()),
            Self::Paused => Err(QueueError::CounterPaused),
        }
    }
}

/// One row of `queue_counters`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counter {
    pub organization_id: String,
    pub branch_id: BranchId,
    /// `users.id` of the teller.
    pub teller_id: String,
    pub status: CounterStatus,
}

impl Counter {
    /// A PAUSED counter cannot call (`QueueError::CounterPaused`, §4.3).
    pub fn can_call(&self) -> Result<(), QueueError> {
        self.status.can_call()
    }
}
