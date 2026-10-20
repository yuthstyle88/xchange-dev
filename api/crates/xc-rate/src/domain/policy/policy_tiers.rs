//! Policy tiers domain entity and validation (rates.md §12.3, §12.10).

use xc_kernel::money::{Money, RatePercent, Side};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;
use crate::domain::publication::value_tier::{TierBand, TierCode};

/// One row of rate policy value tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyTier {
    pub tier_no: u8,
    pub upper_inclusive_thb: Option<Money>,
    pub buy_margin_pct: RatePercent,
    pub sell_margin_pct: RatePercent,
}

impl PolicyTier {
    pub fn new(
        tier_no: u8,
        upper_inclusive_thb: Option<Money>,
        buy_margin_pct: RatePercent,
        sell_margin_pct: RatePercent,
    ) -> Self {
        Self {
            tier_no,
            upper_inclusive_thb,
            buy_margin_pct,
            sell_margin_pct,
        }
    }
}

/// Validated set of four policy tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyTiers {
    tiers: Vec<PolicyTier>,
}

impl PolicyTiers {
    /// Validates exactly four tiers 1..=4 with strictly rising upper bounds > 0,
    /// where tier 4 alone has no upper bound.
    pub fn new(mut rows: Vec<PolicyTier>) -> Result<Self, RateEngineError> {
        if rows.len() != 4 {
            return Err(RateEngineError::ValidationFailed(format!(
                "exactly four policy tier rows are required, got {}",
                rows.len()
            )));
        }

        rows.sort_by_key(|t| t.tier_no);

        for (idx, tier) in rows.iter().enumerate() {
            let expected_tier_no = (idx + 1) as u8;
            if tier.tier_no != expected_tier_no {
                return Err(RateEngineError::ValidationFailed(format!(
                    "tier_no must be 1..=4, got {}",
                    tier.tier_no
                )));
            }
        }

        if rows[3].upper_inclusive_thb.is_some() {
            return Err(RateEngineError::ValidationFailed(
                "tier 4 must have no upper bound (open above)".to_string(),
            ));
        }

        let mut prev_upper = Money::ZERO;
        for (i, row) in rows.iter().enumerate().take(3) {
            let upper = match row.upper_inclusive_thb {
                Some(m) => m,
                None => {
                    return Err(RateEngineError::ValidationFailed(format!(
                        "tier {} must have an upper bound",
                        i + 1
                    )));
                }
            };
            if upper <= Money::ZERO {
                return Err(RateEngineError::ValidationFailed(format!(
                    "tier {} upper bound must be strictly greater than 0, got {}",
                    i + 1,
                    upper
                )));
            }
            if upper <= prev_upper {
                return Err(RateEngineError::ValidationFailed(format!(
                    "tier {} upper bound ({}) must be strictly rising above previous ({})",
                    i + 1,
                    upper,
                    prev_upper
                )));
            }
            prev_upper = upper;
        }

        Ok(Self { tiers: rows })
    }

    /// Returns the four TierBands with codes T1..T4.
    pub fn bands(&self) -> [TierBand; 4] {
        [
            TierBand {
                code: TierCode::new("T1").expect("valid tier code"),
                lower_exclusive_thb: None,
                upper_inclusive_thb: self.tiers[0].upper_inclusive_thb,
            },
            TierBand {
                code: TierCode::new("T2").expect("valid tier code"),
                lower_exclusive_thb: self.tiers[0].upper_inclusive_thb,
                upper_inclusive_thb: self.tiers[1].upper_inclusive_thb,
            },
            TierBand {
                code: TierCode::new("T3").expect("valid tier code"),
                lower_exclusive_thb: self.tiers[1].upper_inclusive_thb,
                upper_inclusive_thb: self.tiers[2].upper_inclusive_thb,
            },
            TierBand {
                code: TierCode::new("T4").expect("valid tier code"),
                lower_exclusive_thb: self.tiers[2].upper_inclusive_thb,
                upper_inclusive_thb: None,
            },
        ]
    }

    /// Returns the margin for tier `n` (1..=4) and `side`.
    pub fn margin(&self, n: u8, side: Side) -> RatePercent {
        let tier = self
            .tiers
            .iter()
            .find(|t| t.tier_no == n)
            .expect("tier_no must be 1..=4");
        match side {
            Side::Buy => tier.buy_margin_pct,
            Side::Sell => tier.sell_margin_pct,
        }
    }

    pub fn tiers(&self) -> &[PolicyTier] {
        &self.tiers
    }

    /// The same bands with `currency`'s margins (pricing-tiers.md §3 "Resolution"): the override's four margins when
    /// it has them, otherwise these (the branch's) margins. `None` returns a copy of `self`.
    pub fn for_currency(&self, o: Option<&CurrencyTierOverride>) -> PolicyTiers {
        let Some(override_tiers) = o.and_then(|ov| ov.tiers.as_ref()) else {
            return self.clone();
        };
        let tiers = self
            .tiers
            .iter()
            .zip(override_tiers.iter())
            .map(|(band, m)| {
                PolicyTier::new(
                    band.tier_no,
                    band.upper_inclusive_thb,
                    m.buy_margin_pct,
                    m.sell_margin_pct,
                )
            })
            .collect();
        PolicyTiers { tiers }
    }
}
