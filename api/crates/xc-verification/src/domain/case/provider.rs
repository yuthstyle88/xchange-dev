//! `Provider` (design V5, OQ-3): reviewer only, final.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Staff,
}

impl Provider {
    /// `STAFF`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Staff => "STAFF",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "STAFF" => Some(Self::Staff),
            _ => None,
        }
    }
}
