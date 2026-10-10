//! R5: the side's relation to the target at the last observation.

/// Stored as `rate_alerts.relation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relation {
    Unknown,
    Satisfied,
    NotSatisfied,
}

impl Relation {
    /// The stored value: `UNKNOWN`, `SATISFIED` or `NOT_SATISFIED`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Satisfied => "SATISFIED",
            Self::NotSatisfied => "NOT_SATISFIED",
        }
    }

    /// Reads the stored value exactly; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "UNKNOWN" => Some(Self::Unknown),
            "SATISFIED" => Some(Self::Satisfied),
            "NOT_SATISFIED" => Some(Self::NotSatisfied),
            _ => None,
        }
    }
}
