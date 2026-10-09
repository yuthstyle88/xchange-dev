//! UserStatus enum for the Blueprint user model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UserStatus {
    Invited,
    Active,
    Disabled,
}

impl UserStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Invited => "INVITED",
            Self::Active => "ACTIVE",
            Self::Disabled => "DISABLED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "INVITED" => Some(Self::Invited),
            "ACTIVE" => Some(Self::Active),
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
