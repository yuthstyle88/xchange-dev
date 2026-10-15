//! `SegmentRule` (design §4.1, §7, D-14).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::metrics_source::CustomerMetrics;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_thb: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookback_months: Option<u32>,
}

impl SegmentRule {
    pub fn matches(&self, metrics: &CustomerMetrics) -> bool {
        if let Some(ref required_tier) = self.tier {
            if metrics.tier_code.as_deref() != Some(required_tier.as_str()) {
                return false;
            }
        }

        if let Some(max_recency_days) = self.recency_days {
            match metrics.recency_days {
                Some(days) if days <= max_recency_days => {}
                _ => return false,
            }
        }

        if let Some(min_frequency) = self.frequency {
            if metrics.frequency < min_frequency {
                return false;
            }
        }

        if let Some(min_value) = self.value_thb {
            if metrics.value_thb < min_value {
                return false;
            }
        }

        if let Some(ref branch) = self.home_branch {
            if metrics.home_branch.as_deref() != Some(branch.as_str()) {
                return false;
            }
        }

        true
    }

    pub fn default_lookback_months() -> u32 {
        12
    }
}
