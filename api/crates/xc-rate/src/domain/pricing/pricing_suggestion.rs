//! V.9's suggested tier prices and the margins that produce them (pricing-tiers.md §5.1, §5.2).
//!
//! SKELETON (PT-0): the types are fixed now so `RowStatus` and the board can use them. `PricingSuggestion::of` is
//! PT-9's and waits for V.9's `runtime/pricing.mjs` (owner P-2, §14 Q-P3). `SuggestionInput` is `[proposed]` until
//! those files are read.

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

/// `[proposed]` What `PricingSuggestion::of` reads; fixed when V.9's files are read.
#[derive(Debug, Clone)]
pub struct SuggestionInput {
    pub currency: CurrencyCode,
    pub quotes: Vec<MarketQuote>,
    /// The draft path's base per side (BoT, else START), at the currency's unit size.
    pub base_buy: Option<Rate>,
    pub base_sell: Option<Rate>,
    pub buy_adjust_pct: RatePercent,
    pub sell_adjust_pct: RatePercent,
    pub unit_size: Decimal,
    pub min_spread_pct: Option<RatePercent>,
    /// The partner cost from the current publication's snapshot (§5.2 `BelowCost`).
    pub partner_buy: Option<Rate>,
    /// Tier 1 of the current publication (§5.2 `LargeMove`).
    pub published_tier1_buy: Option<Rate>,
    pub published_tier1_sell: Option<Rate>,
}

pub struct PricingSuggestion;

impl PricingSuggestion {
    pub fn of(_input: &SuggestionInput) -> Suggestion {
        todo!("PT-9: PricingSuggestion::of waits for V.9's runtime/pricing.mjs (P-2)")
    }
}
