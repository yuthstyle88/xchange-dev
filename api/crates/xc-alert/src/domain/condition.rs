//! R3: the two conditions of an alert.

use rust_decimal::Decimal;

/// `ABOVE` (rate ≥ target) or `BELOW` (rate ≤ target). Stored as `rate_alerts.condition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertCondition {
    Above,
    Below,
}

impl AlertCondition {
    /// Whether `rate` (THB per `unit_size`) satisfies the condition against `target` (same unit).
    pub fn satisfied(self, rate: Decimal, target: Decimal) -> bool {
        match self {
            Self::Above => rate >= target,
            Self::Below => rate <= target,
        }
    }

    /// The stored value: `ABOVE` or `BELOW`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Above => "ABOVE",
            Self::Below => "BELOW",
        }
    }

    /// Reads the stored value exactly; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ABOVE" => Some(Self::Above),
            "BELOW" => Some(Self::Below),
            _ => None,
        }
    }
}
