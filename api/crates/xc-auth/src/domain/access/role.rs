use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Executive,
    Admin,
    RateManager,
    BranchManager,
    Teller,
    Finance,
    Marketing,
    Customer,
    VerificationReviewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Executive => "EXECUTIVE",
            Role::Admin => "ADMIN",
            Role::RateManager => "RATE_MANAGER",
            Role::BranchManager => "BRANCH_MANAGER",
            Role::Teller => "TELLER",
            Role::Finance => "FINANCE",
            Role::Marketing => "MARKETING",
            Role::Customer => "CUSTOMER",
            Role::VerificationReviewer => "VERIFICATION_REVIEWER",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "EXECUTIVE" => Some(Role::Executive),
            "ADMIN" => Some(Role::Admin),
            "RATE_MANAGER" => Some(Role::RateManager),
            "BRANCH_MANAGER" => Some(Role::BranchManager),
            "TELLER" => Some(Role::Teller),
            "FINANCE" => Some(Role::Finance),
            "MARKETING" => Some(Role::Marketing),
            "CUSTOMER" => Some(Role::Customer),
            "VERIFICATION_REVIEWER" => Some(Role::VerificationReviewer),
            _ => None,
        }
    }
}
