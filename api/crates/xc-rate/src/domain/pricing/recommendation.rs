//! V.9's owner recommendations `recommend` (`engine.mjs:154-198`; pricing-tiers.md §5.2).
//!
//! SKELETON (XC-PRICING PX-0): `Recommendation::list` is PX-8's. Types and signature stay.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, RoundingStrategy};

use crate::domain::pricing::market_quote::{MarketQuote, MarketSource};

/// Sorted in this order (`engine.mjs:196`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    High,
    Warn,
    Medium,
    Low,
}

/// The fixed set the FE labels (pricing-tiers.md §5.2 table). `Debug` is the wire text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecommendationCode {
    TtStale,
    SrStale,
    SellAboveMarket,
    SellBelowMarket,
    BuyBelowMarket,
    BuyAboveMarket,
    SpreadWide,
    BelowPartnerCost,
    MarketMovedSinceUpdate,
    NotOffered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecommendationSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub severity: Severity,
    pub code: RecommendationCode,
    pub currency: Option<String>,
    pub side: Option<RecommendationSide>,
    /// The gap in % against the anchor, rounded to 2 places, sign kept (market-gap codes only).
    pub gap_pct: Option<Decimal>,
    /// The anchor price V.9 offers as the fix (market-gap codes only), per one unit.
    pub value: Option<Decimal>,
    /// The currency's THB volume over the last 7 days (0 when none), used for the order.
    pub volume_thb: Decimal,
}

/// One enabled currency's counter rate, per one unit.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterRate {
    pub currency: String,
    pub buy: Decimal,
    pub sell: Decimal,
    pub partner_cost: Option<Decimal>,
    /// When the counter rate was published (V.9's `updated`).
    pub published_at: Option<DateTime<Utc>>,
    /// The currency's anchor (`MarketQuote::anchor_for`), per one unit; `None` ⇒ no market rows for it.
    pub anchor: Option<MarketQuote>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecommendationInput {
    pub tt_stale: bool,
    pub sr_stale: bool,
    /// TT's `source_updated_at` (V.9 `market.TT.sourceUpdatedAt`).
    pub tt_source_updated_at: Option<DateTime<Utc>>,
    pub counters: Vec<CounterRate>,
    /// Currencies TT and Super Rich both list.
    pub listed_by_both: Vec<String>,
    /// Currencies the branch sells.
    pub enabled: HashSet<String>,
    /// THB volume per currency over the last 7 days (non-cancelled, LIVE).
    pub volume_thb: HashMap<String, Decimal>,
}

impl Recommendation {
    /// Every recommendation, highest severity first, then the largest 7-day volume (`engine.mjs:196-197`).
    pub fn list(input: &RecommendationInput) -> Vec<Recommendation> {
        let mut out = Vec::new();
        let plain = |severity, code, currency: Option<String>, volume_thb| Recommendation {
            severity,
            code,
            currency,
            side: None,
            gap_pct: None,
            value: None,
            volume_thb,
        };
        if input.tt_stale {
            out.push(plain(
                Severity::Warn,
                RecommendationCode::TtStale,
                None,
                Decimal::ZERO,
            ));
        }
        if input.sr_stale {
            out.push(plain(
                Severity::Warn,
                RecommendationCode::SrStale,
                None,
                Decimal::ZERO,
            ));
        }

        let far = Decimal::ONE;
        let two = Decimal::from(2);
        let one_half = Decimal::new(15, 1);
        let half = Decimal::new(5, 1);
        let point_two = Decimal::new(2, 1);

        for c in &input.counters {
            if !input.enabled.contains(&c.currency)
                || c.buy <= Decimal::ZERO
                || c.sell <= Decimal::ZERO
            {
                continue;
            }
            let Some(anchor) = &c.anchor else {
                continue;
            };
            let (Some(a_buy_rate), Some(a_sell_rate)) = (anchor.buy, anchor.sell) else {
                continue;
            };
            let a_buy = a_buy_rate.value().normalize();
            let a_sell = a_sell_rate.value().normalize();
            if a_buy <= Decimal::ZERO || a_sell <= Decimal::ZERO {
                continue;
            }
            let vol = input
                .volume_thb
                .get(&c.currency)
                .copied()
                .unwrap_or(Decimal::ZERO);

            let sell_gap = (c.sell / a_sell - Decimal::ONE) * Decimal::ONE_HUNDRED;
            let buy_gap = (c.buy / a_buy - Decimal::ONE) * Decimal::ONE_HUNDRED;
            let round2 =
                |d: Decimal| d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);

            if sell_gap > far {
                out.push(Recommendation {
                    severity: Severity::High,
                    code: RecommendationCode::SellAboveMarket,
                    currency: Some(c.currency.clone()),
                    side: Some(RecommendationSide::Sell),
                    gap_pct: Some(round2(sell_gap)),
                    value: Some(a_sell),
                    volume_thb: vol,
                });
            } else if sell_gap < -far {
                out.push(Recommendation {
                    severity: Severity::Medium,
                    code: RecommendationCode::SellBelowMarket,
                    currency: Some(c.currency.clone()),
                    side: Some(RecommendationSide::Sell),
                    gap_pct: Some(round2(sell_gap)),
                    value: Some(a_sell),
                    volume_thb: vol,
                });
            }

            if buy_gap < -far {
                out.push(Recommendation {
                    severity: Severity::High,
                    code: RecommendationCode::BuyBelowMarket,
                    currency: Some(c.currency.clone()),
                    side: Some(RecommendationSide::Buy),
                    gap_pct: Some(round2(buy_gap)),
                    value: Some(a_buy),
                    volume_thb: vol,
                });
            } else if buy_gap > far {
                out.push(Recommendation {
                    severity: Severity::Medium,
                    code: RecommendationCode::BuyAboveMarket,
                    currency: Some(c.currency.clone()),
                    side: Some(RecommendationSide::Buy),
                    gap_pct: Some(round2(buy_gap)),
                    value: Some(a_buy),
                    volume_thb: vol,
                });
            }

            let shop_spread = (c.sell - c.buy) / ((c.sell + c.buy) / two) * Decimal::ONE_HUNDRED;
            let anchor_spread = (a_sell - a_buy) / ((a_sell + a_buy) / two) * Decimal::ONE_HUNDRED;
            if shop_spread > one_half * anchor_spread && shop_spread - anchor_spread > half {
                out.push(plain(
                    Severity::Medium,
                    RecommendationCode::SpreadWide,
                    Some(c.currency.clone()),
                    vol,
                ));
            }

            if let Some(cost) = c.partner_cost
                && cost > Decimal::ZERO
                && (c.sell < cost || c.buy > cost)
            {
                out.push(plain(
                    Severity::High,
                    RecommendationCode::BelowPartnerCost,
                    Some(c.currency.clone()),
                    vol,
                ));
            }

            if anchor.source == MarketSource::Tt
                && let (Some(tt_at), Some(pub_at)) = (input.tt_source_updated_at, c.published_at)
                && tt_at > pub_at
                && (sell_gap.abs() > point_two || buy_gap.abs() > point_two)
            {
                out.push(plain(
                    Severity::Low,
                    RecommendationCode::MarketMovedSinceUpdate,
                    Some(c.currency.clone()),
                    vol,
                ));
            }
        }

        for code in &input.listed_by_both {
            if !input.enabled.contains(code) {
                let vol = input.volume_thb.get(code).copied().unwrap_or(Decimal::ZERO);
                out.push(plain(
                    Severity::Low,
                    RecommendationCode::NotOffered,
                    Some(code.clone()),
                    vol,
                ));
            }
        }

        out.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then_with(|| b.volume_thb.cmp(&a.volume_thb))
        });
        out
    }
}
