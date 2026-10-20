//! One currency's override of a policy version's tier margins (pricing-tiers.md §3, owner P-1: per currency, default
//! from the branch). Bands (THB bounds) are never overridden: only margins, the minimum spread and the anchor.
//!
//! SKELETON (PT-0): `CurrencyTierOverride::new` is PT-2's. The types and signatures stay.

use xc_kernel::money::{CurrencyCode, RatePercent};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::anchor_source::AnchorSource;

/// One tier's margins for one currency: `rate_policy_currency_tiers` (`tier_no` 1..=4, signed, scale 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierMargin {
    pub tier_no: u8,
    pub buy_margin_pct: RatePercent,
    pub sell_margin_pct: RatePercent,
}

/// A currency's settings in one policy version: its four margins (or none), its minimum spread, its anchor.
///
/// `tiers` is `None` for a currency that overrides only the minimum spread or the anchor
/// (`rate_policy_currency_settings` row without `rate_policy_currency_tiers` rows, §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyTierOverride {
    pub currency: CurrencyCode,
    pub tiers: Option<[TierMargin; 4]>,
    pub min_spread_pct: Option<RatePercent>,
    pub anchor_source: Option<AnchorSource>,
}

impl CurrencyTierOverride {
    /// Builds one currency's override. `margins` must be empty (no margin override) or hold exactly the four tiers
    /// 1..=4, each once (§3 "all four or none"); anything else is `ValidationFailed`. `min_spread_pct`, when set,
    /// lies in 0..=5 (the column's CHECK).
    pub fn new(
        _currency: CurrencyCode,
        _margins: Vec<TierMargin>,
        _min_spread_pct: Option<RatePercent>,
        _anchor_source: Option<AnchorSource>,
    ) -> Result<CurrencyTierOverride, RateEngineError> {
        todo!("PT-2: CurrencyTierOverride::new")
    }

    /// The margin of tier `n` (1..=4) for `side`, if this currency overrides margins.
    pub fn margin(&self, n: u8, side: xc_kernel::money::Side) -> Option<RatePercent> {
        let tiers = self.tiers.as_ref()?;
        let t = tiers.iter().find(|t| t.tier_no == n)?;
        Some(match side {
            xc_kernel::money::Side::Buy => t.buy_margin_pct,
            xc_kernel::money::Side::Sell => t.sell_margin_pct,
        })
    }
}
