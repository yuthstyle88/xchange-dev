//! V.9's suggested tier prices and the margins that produce them (pricing-tiers.md §5.1, §5.2).
//!
//! SKELETON (PT-0, then XC-PRICING PX-0): `PricingSuggestion::of` is PX-6's. `SuggestionInput` and
//! `SuggestionConfig` are fixed by §5.1/§5.2 from V.9's `src/pricing/engine.mjs` (`suggestTiers`, `:40-65`).

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};

use crate::domain::policy::currency_tier_override::TierMargin;
use crate::domain::pricing::market_quote::MarketQuote;
use crate::domain::pricing::pricing_flag::PricingFlag;

/// One suggested tier price (buy and sell), per the currency's unit size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierPrice {
    pub tier_no: u8,
    pub buy: Option<Rate>,
    pub sell: Option<Rate>,
}

/// The suggestion for one currency. With no anchor, `tiers` and `margins` are `None` and `flags` holds `NoAnchor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub tiers: Option<[TierPrice; 4]>,
    pub margins: Option<[TierMargin; 4]>,
    pub flags: Vec<PricingFlag>,
}

impl Suggestion {
    /// True when a flag keeps the row out of bulk approve.
    pub fn blocked(&self) -> bool {
        self.flags.iter().any(|f| f.blocking())
    }
}

/// V.9's `DEFAULT_CONFIG` (`engine.mjs:13`): `better` is the % better for the customer than the anchor, per tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuggestionConfig {
    pub better: [Decimal; 4],
    pub min_spread_pct: Decimal,
    pub far_from_anchor_pct: Decimal,
    pub large_move_pct: Decimal,
}

impl SuggestionConfig {
    /// `{better: [0.1, 0.14, 0.2, 0.3], minSpreadPct: 0.3, farFromAnchorPct: 1, largeMovePct: 1}`.
    pub fn v9_default() -> Self {
        Self {
            better: [
                Decimal::new(10, 2),
                Decimal::new(14, 2),
                Decimal::new(20, 2),
                Decimal::new(30, 2),
            ],
            min_spread_pct: Decimal::new(3, 1),
            far_from_anchor_pct: Decimal::ONE,
            large_move_pct: Decimal::ONE,
        }
    }
}

impl Default for SuggestionConfig {
    fn default() -> Self {
        Self::v9_default()
    }
}

/// What `PricingSuggestion::of` reads (pricing-tiers.md §5.1).
#[derive(Debug, Clone)]
pub struct SuggestionInput {
    pub currency: CurrencyCode,
    /// The currency's market quotes, per one unit (`MarketQuote::unit_size` 1).
    pub quotes: Vec<MarketQuote>,
    /// TT's `decimals` hint for this currency (`engine.mjs:19-22`); Super Rich has none.
    pub tt_decimals: Option<u32>,
    /// The currency's unit size: tier prices, bases and published rates are at this size.
    pub unit_size: Decimal,
    /// The draft path's base per side (BoT, else START, or the §3.1 anchor), at the unit size.
    pub base_buy: Option<Rate>,
    pub base_sell: Option<Rate>,
    pub buy_adjust_pct: RatePercent,
    pub sell_adjust_pct: RatePercent,
    /// The partner cost per one unit (`source_snapshot.partner.cost`, §5.2 `BelowCost`).
    pub partner_cost: Option<Decimal>,
    /// Tier 1 of the current publication, at the unit size (§5.2 `LargeMove`).
    pub published_tier1_buy: Option<Rate>,
    pub published_tier1_sell: Option<Rate>,
    /// `min_spread_pct` already resolved by the caller (the currency's setting, else V.9's 0.3).
    pub config: SuggestionConfig,
}

pub struct PricingSuggestion;

impl PricingSuggestion {
    pub fn of(_input: &SuggestionInput) -> Suggestion {
        todo!("PX-6: PricingSuggestion::of (V.9 suggestTiers)")
    }
}
