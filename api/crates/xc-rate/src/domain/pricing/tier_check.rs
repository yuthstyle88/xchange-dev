//! V.9's approval check `validateTiers` (`engine.mjs:68-84`; pricing-tiers.md §5.2).
//!
//! SKELETON (XC-PRICING PX-0): `TierCheck::validate` is PX-7's. Types and signature stay.

use rust_decimal::Decimal;

use crate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use crate::domain::pricing::pricing_suggestion::{SuggestionConfig, TierPrice};

/// One reason a tier set cannot be approved. `tier` is 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierError {
    /// Not exactly 4 tiers.
    Count,
    /// A missing or non-positive buy or sell.
    NotPositive { tier: u8 },
    /// `sell <= buy`.
    SellNotAboveBuy { tier: u8 },
    /// Equal-or-better: this tier's buy is below the previous tier's.
    BuyBelowPrevious { tier: u8 },
    /// Equal-or-better: this tier's sell is above the previous tier's.
    SellAbovePrevious { tier: u8 },
    /// Tier 1 is more than `limit_pct` (3 × `far_from_anchor_pct`) from the anchor on a side.
    FarFromAnchor {
        source: MarketSource,
        limit_pct: Decimal,
    },
}

pub struct TierCheck;

impl TierCheck {
    /// `tiers` are at the currency's `unit_size`; `anchor` is per one unit (a market quote). Every error found, in
    /// tier order; the anchor check runs only when no other error was found (`engine.mjs:78`).
    pub fn validate(
        tiers: &[TierPrice],
        unit_size: Decimal,
        anchor: Option<&MarketQuote>,
        config: &SuggestionConfig,
    ) -> Vec<TierError> {
        let _ = (tiers, unit_size, anchor, config);
        todo!("PX-7: TierCheck::validate")
    }
}
