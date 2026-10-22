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
        if tiers.len() != 4 {
            return vec![TierError::Count];
        }

        let mut errors = Vec::new();
        for (idx, t) in tiers.iter().enumerate() {
            let tier = (idx + 1) as u8;
            let buy = t.buy.map(|r| r.value()).filter(|&v| v > Decimal::ZERO);
            let sell = t.sell.map(|r| r.value()).filter(|&v| v > Decimal::ZERO);
            match (buy, sell) {
                (Some(b), Some(s)) => {
                    if s <= b {
                        errors.push(TierError::SellNotAboveBuy { tier });
                    }
                }
                _ => {
                    errors.push(TierError::NotPositive { tier });
                }
            }

            if idx > 0 {
                let prev = &tiers[idx - 1];
                if let (Some(b), Some(prev_b)) = (buy, prev.buy.map(|r| r.value()))
                    && prev_b > Decimal::ZERO
                    && b < prev_b
                {
                    errors.push(TierError::BuyBelowPrevious { tier });
                }
                if let (Some(s), Some(prev_s)) = (sell, prev.sell.map(|r| r.value()))
                    && prev_s > Decimal::ZERO
                    && s > prev_s
                {
                    errors.push(TierError::SellAbovePrevious { tier });
                }
            }
        }

        if errors.is_empty()
            && unit_size > Decimal::ZERO
            && let Some(a) = anchor
            && let (Some(a_buy), Some(a_sell), Some(t1_buy), Some(t1_sell)) =
                (a.buy, a.sell, tiers[0].buy, tiers[0].sell)
            && a_buy.value() > Decimal::ZERO
            && a_sell.value() > Decimal::ZERO
        {
            let limit = config.far_from_anchor_pct * Decimal::from(3);
            let hundred = Decimal::from(100);
            let u_buy = t1_buy.value() / unit_size;
            let u_sell = t1_sell.value() / unit_size;
            let buy_diff = ((u_buy / a_buy.value() - Decimal::ONE) * hundred).abs();
            let sell_diff = ((u_sell / a_sell.value() - Decimal::ONE) * hundred).abs();
            if buy_diff > limit || sell_diff > limit {
                errors.push(TierError::FarFromAnchor {
                    source: a.source,
                    limit_pct: limit,
                });
            }
        }

        errors
    }
}
