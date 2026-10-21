//! V.9's suggested tier prices and the margins that produce them (pricing-tiers.md §5.1, §5.2).
//!
//! SKELETON (PT-0, then XC-PRICING PX-0): `PricingSuggestion::of` is PX-6's. `SuggestionInput` and
//! `SuggestionConfig` are fixed by §5.1/§5.2 from V.9's `src/pricing/engine.mjs` (`suggestTiers`, `:40-65`).

use rust_decimal::{Decimal, RoundingStrategy};
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};

use crate::domain::policy::currency_tier_override::TierMargin;
use crate::domain::pricing::market_quote::{MarketQuote, MarketSource};
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
    pub fn of(input: &SuggestionInput) -> Suggestion {
        let (Some(anchor), mut flags) = MarketQuote::anchor_for(&input.quotes) else {
            return Suggestion {
                tiers: None,
                margins: None,
                flags: vec![PricingFlag::NoAnchor],
            };
        };
        let (Some(anchor_buy_rate), Some(anchor_sell_rate)) = (anchor.buy, anchor.sell) else {
            return Suggestion {
                tiers: None,
                margins: None,
                flags: vec![PricingFlag::NoAnchor],
            };
        };
        let anchor_buy = anchor_buy_rate.value();
        let anchor_sell = anchor_sell_rate.value();

        let d = match (anchor.source, input.tt_decimals) {
            (MarketSource::Tt, Some(hint)) if hint >= 2 => hint.min(4),
            _ if anchor_buy >= Decimal::from(10) => 2,
            _ if anchor_buy >= Decimal::ONE => 3,
            _ => 4,
        };

        let floor_d =
            |v: Decimal| v.round_dp_with_strategy(d, RoundingStrategy::ToNegativeInfinity);
        let ceil_d = |v: Decimal| v.round_dp_with_strategy(d, RoundingStrategy::ToPositiveInfinity);
        let round_d =
            |v: Decimal| v.round_dp_with_strategy(d, RoundingStrategy::MidpointAwayFromZero);
        let hundred = Decimal::from(100);
        let two = Decimal::from(2);

        let mid = (anchor_buy + anchor_sell) / two;
        let min_spread = mid * input.config.min_spread_pct / hundred;

        let mut unit_tiers = [(Decimal::ZERO, Decimal::ZERO); 4];
        let mut prev: Option<(Decimal, Decimal)> = None;
        let mut min_spread_hit = false;

        for (idx, &b) in input.config.better.iter().enumerate() {
            let mut buy = floor_d(anchor_buy * (Decimal::ONE + b / hundred));
            let mut sell = ceil_d(anchor_sell * (Decimal::ONE - b / hundred));
            if sell - buy < min_spread {
                buy = floor_d(mid - min_spread / two);
                sell = ceil_d(mid + min_spread / two);
                min_spread_hit = true;
            }
            if let Some((prev_buy, prev_sell)) = prev {
                buy = buy.max(prev_buy);
                sell = sell.min(prev_sell);
            }
            prev = Some((buy, sell));
            unit_tiers[idx] = (buy, sell);
        }

        if min_spread_hit {
            flags.push(PricingFlag::MinSpread);
        }

        let unrounded_t1_buy = anchor_buy * (Decimal::ONE + input.config.better[0] / hundred);
        let unrounded_t1_sell = anchor_sell * (Decimal::ONE - input.config.better[0] / hundred);
        let pct_diff = |a: Decimal, b: Decimal| ((a - b) / b * hundred).abs();
        let mut precision_hit = pct_diff(unit_tiers[0].0, unrounded_t1_buy) > Decimal::ONE
            || pct_diff(unit_tiers[0].1, unrounded_t1_sell) > Decimal::ONE;

        let tiers = std::array::from_fn(|idx| {
            let (unit_buy, unit_sell) = unit_tiers[idx];
            let buy_at_unit = unit_buy * input.unit_size;
            let sell_at_unit = unit_sell * input.unit_size;
            TierPrice {
                tier_no: (idx + 1) as u8,
                buy: Rate::parse(&buy_at_unit.to_string()).ok(),
                sell: Rate::parse(&sell_at_unit.to_string()).ok(),
            }
        });

        let margins = if let (Some(base_buy), Some(base_sell)) = (input.base_buy, input.base_sell) {
            let base_buy_val = base_buy.value();
            let base_sell_val = base_sell.value();
            let buy_adj = input.buy_adjust_pct.value();
            let sell_adj = input.sell_adjust_pct.value();

            let arr = std::array::from_fn(|idx| {
                let (unit_buy, unit_sell) = unit_tiers[idx];
                let price_buy = unit_buy * input.unit_size;
                let price_sell = unit_sell * input.unit_size;

                let raw_buy_margin = ((price_buy / base_buy_val - Decimal::ONE) * hundred
                    - buy_adj)
                    .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero);
                let raw_sell_margin = ((price_sell / base_sell_val - Decimal::ONE) * hundred
                    - sell_adj)
                    .round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero);

                let buy_margin_pct =
                    RatePercent::parse(&raw_buy_margin.to_string()).unwrap_or(RatePercent::ZERO);
                let sell_margin_pct =
                    RatePercent::parse(&raw_sell_margin.to_string()).unwrap_or(RatePercent::ZERO);

                let repro_buy = round_d(
                    base_buy_val * (Decimal::ONE + (buy_adj + buy_margin_pct.value()) / hundred)
                        / input.unit_size,
                );
                let repro_sell = round_d(
                    base_sell_val * (Decimal::ONE + (sell_adj + sell_margin_pct.value()) / hundred)
                        / input.unit_size,
                );
                if repro_buy != unit_buy || repro_sell != unit_sell {
                    precision_hit = true;
                }

                TierMargin {
                    tier_no: (idx + 1) as u8,
                    buy_margin_pct,
                    sell_margin_pct,
                }
            });
            Some(arr)
        } else {
            None
        };

        if precision_hit {
            flags.push(PricingFlag::Precision);
        }

        if let Some(cost) = input.partner_cost
            && cost > Decimal::ZERO
            && unit_tiers
                .iter()
                .any(|&(buy, sell)| sell < cost || buy > cost)
        {
            flags.push(PricingFlag::BelowCost);
        }

        if let Some(published) = input.published_tier1_buy {
            let pub_val = published.value();
            if pub_val > Decimal::ZERO {
                let tier1_buy_at_unit = unit_tiers[0].0 * input.unit_size;
                if ((tier1_buy_at_unit / pub_val - Decimal::ONE) * hundred).abs()
                    > input.config.large_move_pct
                {
                    flags.push(PricingFlag::LargeMove);
                }
            }
        }

        if margins.is_none() && !flags.contains(&PricingFlag::NoAnchor) {
            flags.push(PricingFlag::NoAnchor);
        }

        Suggestion {
            tiers: Some(tiers),
            margins,
            flags,
        }
    }
}
