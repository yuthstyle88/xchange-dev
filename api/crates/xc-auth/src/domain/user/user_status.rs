//! UserStatus enum for the Blueprint user model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UserStatus {
    Pending,
    Active,
    Suspended,
    Disabled,
}

impl UserStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Active => "ACTIVE",
            Self::Suspended => "SUSPENDED",
            Self::Disabled => "DISABLED",
        }
    }

    /// The §3.2 status table (7 allowed moves). RBAC function ticket.
    pub fn may_move(self, to: UserStatus) -> bool {
        let _ = to;
        todo!("RBAC function ticket: UserStatus::may_move (§3.2)")
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "PENDING" => Some(Self::Pending),
            "ACTIVE" => Some(Self::Active),
            "SUSPENDED" => Some(Self::Suspended),
            "DISABLED" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
