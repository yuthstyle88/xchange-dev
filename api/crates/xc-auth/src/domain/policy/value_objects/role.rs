//! Role value object.

/// A role an actor can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Owner,
    Front,
    Backend,
    Manager,
    Rates,
    Monitor,
    Finance,
    System,
    Documents,
    DocumentReview,
}

impl Role {
    /// The nine roles an employee can hold, in this order.
    pub const STAFF: [Role; 9] = [
        Role::Front,
        Role::Backend,
        Role::Manager,
        Role::Rates,
        Role::Monitor,
        Role::Finance,
        Role::System,
        Role::Documents,
        Role::DocumentReview,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "OWNER",
            Role::Front => "FRONT",
            Role::Backend => "BACKEND",
            Role::Manager => "MANAGER",
            Role::Rates => "RATES",
            Role::Monitor => "MONITOR",
            Role::Finance => "FINANCE",
            Role::System => "SYSTEM",
            Role::Documents => "DOCUMENTS",
            Role::DocumentReview => "DOCUMENT_REVIEW",
        }
    }

    pub fn parse(text: &str) -> Option<Role> {
        match text {
            "OWNER" => Some(Role::Owner),
            "FRONT" => Some(Role::Front),
            "BACKEND" => Some(Role::Backend),
            "MANAGER" => Some(Role::Manager),
            "RATES" => Some(Role::Rates),
            "MONITOR" => Some(Role::Monitor),
            "FINANCE" => Some(Role::Finance),
            "SYSTEM" => Some(Role::System),
            "DOCUMENTS" => Some(Role::Documents),
            "DOCUMENT_REVIEW" => Some(Role::DocumentReview),
            _ => None,
        }
    }
}
