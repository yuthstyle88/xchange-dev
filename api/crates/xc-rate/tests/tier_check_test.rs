//! XC-PRICING PX-7 (pricing-tiers.md §5.2): `TierCheck::validate`, ported from V.9 `test/pricing.mjs:102-107`
//! (`validateTiers`). Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not
//! edit, delete or weaken; add tests in a file of your own. Pure.

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::Rate;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_suggestion::{SuggestionConfig, TierPrice};
use xc_rate::domain::pricing::tier_check::{TierCheck, TierError};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn tier(n: u8, buy: &str, sell: &str) -> TierPrice {
    TierPrice { tier_no: n, buy: Some(Rate::parse(buy).unwrap()), sell: Some(Rate::parse(sell).unwrap()) }
}

/// V.9's USD suggestion (`test/pricing.mjs:83`).
fn usd() -> Vec<TierPrice> {
    vec![tier(1, "33.23", "33.87"), tier(2, "33.24", "33.86"), tier(3, "33.26", "33.84"), tier(4, "33.29", "33.80")]
}

fn tt() -> MarketQuote {
    MarketQuote {
        source: MarketSource::Tt,
        buy: Some(Rate::parse("33.2").unwrap()),
        sell: Some(Rate::parse("33.9").unwrap()),
        unit_size: Decimal::ONE,
        fetched_at: Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap(),
        stale: false,
    }
}

fn check(tiers: &[TierPrice], anchor: Option<&MarketQuote>) -> Vec<TierError> {
    TierCheck::validate(tiers, Decimal::ONE, anchor, &SuggestionConfig::v9_default())
}

#[test]
fn the_suggestion_passes() {
    assert_eq!(check(&usd(), Some(&tt())), vec![]);
}

#[test]
fn sell_must_be_above_buy() {
    let mut t = usd();
    t[0] = tier(1, "33", "33");
    assert!(check(&t, None).contains(&TierError::SellNotAboveBuy { tier: 1 }));
}

#[test]
fn a_higher_tier_may_not_be_worse() {
    let t = vec![tier(1, "33.24", "33.86"), tier(2, "33.23", "33.87"), tier(3, "33.26", "33.84"), tier(4, "33.29", "33.80")];
    let e = check(&t, None);
    assert!(e.contains(&TierError::BuyBelowPrevious { tier: 2 }), "{e:?}");
    assert!(e.contains(&TierError::SellAbovePrevious { tier: 2 }), "{e:?}");
}

#[test]
fn equal_tiers_are_allowed() {
    let t = vec![tier(1, "33.2", "33.9"), tier(2, "33.2", "33.9"), tier(3, "33.2", "33.9"), tier(4, "33.2", "33.9")];
    assert_eq!(check(&t, Some(&tt())), vec![]);
}

#[test]
fn tier_one_far_from_the_anchor_is_refused() {
    let t = vec![tier(1, "30", "31"), tier(2, "30", "31"), tier(3, "30", "31"), tier(4, "30", "31")];
    assert_eq!(
        check(&t, Some(&tt())),
        vec![TierError::FarFromAnchor { source: MarketSource::Tt, limit_pct: d("3") }]
    );
    assert_eq!(check(&t, None), vec![], "no anchor, no distance check");
}

#[test]
fn the_distance_check_waits_for_the_other_checks() {
    let t = vec![tier(1, "30", "30"), tier(2, "30", "31"), tier(3, "30", "31"), tier(4, "30", "31")];
    let e = check(&t, Some(&tt()));
    assert!(!e.iter().any(|x| matches!(x, TierError::FarFromAnchor { .. })), "{e:?}");
}

#[test]
fn four_tiers_exactly() {
    assert_eq!(check(&usd()[..2], None), vec![TierError::Count]);
}

#[test]
fn a_missing_price_is_not_positive() {
    let mut t = usd();
    t[2].sell = None;
    assert!(check(&t, None).contains(&TierError::NotPositive { tier: 3 }));
}

#[test]
fn the_anchor_is_compared_per_unit() {
    let t: Vec<TierPrice> = vec![tier(1, "3323", "3387"), tier(2, "3324", "3386"), tier(3, "3326", "3384"), tier(4, "3329", "3380")];
    assert_eq!(TierCheck::validate(&t, d("100"), Some(&tt()), &SuggestionConfig::v9_default()), vec![]);
}
