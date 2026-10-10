//! R8: which published sides an alert observes — one explicit policy, in this file only.

use rust_decimal::Decimal;
use xc_rate::domain::publication::freshness::Freshness;

/// `rate_alert_settings.observe_policy` (Q-R3 decided `ALL_FRESH`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObservePolicy {
    /// AUTO and MANUAL sides alike; a stale side is skipped.
    AllFresh,
    /// Every side, stale included.
    All,
    /// Only AUTO sides that are fresh.
    AutoFresh,
}

impl ObservePolicy {
    /// The stored value: `ALL_FRESH`, `ALL` or `AUTO_FRESH`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllFresh => "ALL_FRESH",
            Self::All => "ALL",
            Self::AutoFresh => "AUTO_FRESH",
        }
    }

    /// Reads the stored value exactly; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ALL_FRESH" => Some(Self::AllFresh),
            "ALL" => Some(Self::All),
            "AUTO_FRESH" => Some(Self::AutoFresh),
            _ => None,
        }
    }
}

/// How the side was priced (`source_snapshot.buy_mode` / `sell_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SideMode {
    Auto,
    Manual,
}

/// One published side as the policy sees it: its rate (THB per the line's `unit_size`) and how it was priced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideSnapshot {
    pub rate: Decimal,
    pub mode: SideMode,
}

/// The policy's answer: compare this rate, or skip the side (the relation stays as it is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    Compare(Decimal),
    Skip,
}

/// The one function deciding whether a published side is observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertSourcePolicy {
    policy: ObservePolicy,
}

impl AlertSourcePolicy {
    pub fn new(policy: ObservePolicy) -> Self {
        Self { policy }
    }

    /// `Compare(side.rate)` when the policy observes this side, `Skip` otherwise.
    pub fn observes(&self, side: &SideSnapshot, freshness: &Freshness) -> Observation {
        match self.policy {
            ObservePolicy::All => Observation::Compare(side.rate),
            ObservePolicy::AllFresh => {
                if freshness.stale {
                    Observation::Skip
                } else {
                    Observation::Compare(side.rate)
                }
            }
            ObservePolicy::AutoFresh => {
                if side.mode == SideMode::Auto && !freshness.stale {
                    Observation::Compare(side.rate)
                } else {
                    Observation::Skip
                }
            }
        }
    }
}
