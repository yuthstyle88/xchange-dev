//! XC-PRICING PX-6 (pricing-tiers.md §5.2): `PricingSuggestion::of`, ported from V.9 `test/pricing.mjs:79-100`
//! (`suggestTiers`), plus §5.2's margin conversion and unit size. Every expected price is V.9's. Written before the
//! implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit, delete or weaken; add tests in a
//! file of your own. Pure: no database, no clock.

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_flag::PricingFlag;
use xc_rate::domain::pricing::pricing_suggestion::{
    PricingSuggestion, Suggestion, SuggestionConfig, SuggestionInput,
};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("rate")
}

fn quote(source: MarketSource, buy: &str, sell: &str) -> MarketQuote {
    MarketQuote {
        source,
        buy: Some(rate(buy)),
        sell: Some(rate(sell)),
        unit_size: Decimal::ONE,
        fetched_at: Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap(),
        stale: false,
    }
}

fn input(code: &str, quotes: Vec<MarketQuote>, tt_decimals: Option<u32>) -> SuggestionInput {
    SuggestionInput {
        currency: CurrencyCode::parse(code).expect("currency"),
        quotes,
        tt_decimals,
        unit_size: Decimal::ONE,
        base_buy: None,
        base_sell: None,
        buy_adjust_pct: RatePercent::ZERO,
        sell_adjust_pct: RatePercent::ZERO,
        partner_cost: None,
        published_tier1_buy: None,
        published_tier1_sell: None,
        config: SuggestionConfig::v9_default(),
    }
}

/// V.9's TT USD(L) and Super Rich USD headlines (`test/pricing.mjs:33-46`), with a BoT base of 33 / 34 so the
/// row has margins (§5.2: no base ⇒ `NoAnchor`).
fn usd() -> SuggestionInput {
    let mut i = input(
        "USD",
        vec![quote(MarketSource::Tt, "33.2", "33.9"), quote(MarketSource::SuperRich, "33.33", "33.40")],
        Some(2),
    );
    i.base_buy = Some(rate("33"));
    i.base_sell = Some(rate("34"));
    i
}

fn prices(s: &Suggestion) -> Vec<(Decimal, Decimal)> {
    s.tiers
        .expect("tiers")
        .iter()
        .map(|t| (t.buy.expect("buy").value(), t.sell.expect("sell").value()))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(Decimal, Decimal)> {
    list.iter().map(|(b, s)| (d(b), d(s))).collect()
}

#[test]
fn usd_default_tiers_are_v9s() {
    let s = PricingSuggestion::of(&usd());
    assert_eq!(
        prices(&s),
        pairs(&[("33.23", "33.87"), ("33.24", "33.86"), ("33.26", "33.84"), ("33.29", "33.80")]),
        "default +0.10/+0.14/+0.20/+0.30 %"
    );
    assert_eq!(s.flags, Vec::<PricingFlag>::new());
    let t = s.tiers.unwrap();
    assert_eq!(t.map(|x| x.tier_no), [1, 2, 3, 4]);
}

#[test]
fn a_zero_tier_one_is_the_anchor_itself() {
    let mut i = usd();
    i.config.better = [d("0"), d("0.1"), d("0.2"), d("0.3")];
    assert_eq!(
        prices(&PricingSuggestion::of(&i)),
        pairs(&[("33.2", "33.9"), ("33.23", "33.87"), ("33.26", "33.84"), ("33.29", "33.80")])
    );
}

#[test]
fn higher_tiers_are_never_worse() {
    for better in [["0.1", "0.14", "0.2", "0.3"], ["0.3", "0.2", "0.1", "0"], ["0.5", "0.5", "0.5", "0.5"]] {
        let mut i = usd();
        i.config.better = better.map(d);
        let p = prices(&PricingSuggestion::of(&i));
        for n in 1..4 {
            assert!(p[n].0 >= p[n - 1].0, "buy tier {} vs {}: {p:?}", n + 1, n);
            assert!(p[n].1 <= p[n - 1].1, "sell tier {} vs {}: {p:?}", n + 1, n);
        }
    }
}

#[test]
fn half_a_percent_better_rounds_buy_down() {
    let mut i = usd();
    i.config.better = [d("0.5"); 4];
    assert_eq!(prices(&PricingSuggestion::of(&i))[0].0, d("33.36"));
}

#[test]
fn gbp_from_super_rich_keeps_the_minimum_spread() {
    let s = PricingSuggestion::of(&input("GBP", vec![quote(MarketSource::SuperRich, "44.40", "44.55")], None));
    assert!(s.flags.contains(&PricingFlag::NoTtQuote));
    assert!(s.flags.contains(&PricingFlag::MinSpread), "higher tiers would cut the spread below 0.3 %");
    for (buy, sell) in prices(&s) {
        let spread = (sell - buy) / ((sell + buy) / d("2")) * d("100");
        assert!(spread >= d("0.3"), "spread {spread} %");
    }
    assert_eq!(prices(&s)[0], (d("44.40"), d("44.55")));
}

#[test]
fn no_quote_is_no_anchor_and_no_tiers() {
    let s = PricingSuggestion::of(&input("CHF", vec![], None));
    assert_eq!(s.flags, vec![PricingFlag::NoAnchor]);
    assert_eq!(s.tiers, None);
    assert_eq!(s.margins, None);
}

#[test]
fn an_invalid_tt_quote_falls_back_to_super_rich() {
    // V.9 anchorFor (engine.mjs:33): a quote counts only with buy > 0 and sell > buy.
    let s = PricingSuggestion::of(&input(
        "USD",
        vec![quote(MarketSource::Tt, "33.9", "33.2"), quote(MarketSource::SuperRich, "33.33", "33.40")],
        Some(2),
    ));
    assert!(s.flags.contains(&PricingFlag::NoTtQuote));
}

#[test]
fn a_stale_anchor_is_flagged() {
    let mut i = usd();
    i.quotes[0].stale = true;
    assert!(PricingSuggestion::of(&i).flags.contains(&PricingFlag::StaleSource));
}

#[test]
fn partner_cost_above_a_tier_sell_is_below_cost() {
    let mut i = usd();
    i.partner_cost = Some(d("33.85"));
    assert!(PricingSuggestion::of(&i).flags.contains(&PricingFlag::BelowCost));
    i.partner_cost = Some(d("33.5"));
    assert!(!PricingSuggestion::of(&i).flags.contains(&PricingFlag::BelowCost), "33.29 ≤ 33.5 ≤ 33.80");
}

#[test]
fn partner_cost_below_a_tier_buy_is_below_cost() {
    let mut i = usd();
    i.partner_cost = Some(d("33.25"));
    assert!(PricingSuggestion::of(&i).flags.contains(&PricingFlag::BelowCost), "tier 3 buys at 33.26 > 33.25");
}

#[test]
fn tier_one_moving_more_than_one_percent_is_a_large_move() {
    let mut i = usd();
    i.published_tier1_buy = Some(rate("32"));
    assert!(PricingSuggestion::of(&i).flags.contains(&PricingFlag::LargeMove));
    i.published_tier1_buy = Some(rate("33.1"));
    assert!(!PricingSuggestion::of(&i).flags.contains(&PricingFlag::LargeMove), "33.23 vs 33.1 is 0.39 %");
}

#[test]
fn yen_keeps_four_decimals() {
    let s = PricingSuggestion::of(&input("JPY", vec![quote(MarketSource::Tt, "0.2085", "0.2175")], Some(4)));
    assert_eq!(prices(&s)[0], (d("0.2087"), d("0.2173")));
    assert!(!s.flags.contains(&PricingFlag::Precision));
}

#[test]
fn yen_at_a_unit_size_of_100_is_the_per_unit_price_times_100() {
    let mut i = input("JPY", vec![quote(MarketSource::Tt, "0.2085", "0.2175")], Some(4));
    i.unit_size = d("100");
    assert_eq!(prices(&PricingSuggestion::of(&i))[0], (d("20.87"), d("21.73")));
}

#[test]
fn rupiah_cannot_be_expressed_in_four_decimals() {
    let s = PricingSuggestion::of(&input("IDR", vec![quote(MarketSource::Tt, "0.00178", "0.0021")], Some(5)));
    assert!(s.flags.contains(&PricingFlag::Precision), "4 decimals cannot express IDR");
}

#[test]
fn margins_over_the_base_reproduce_the_tier_prices() {
    let s = PricingSuggestion::of(&usd());
    let m = s.margins.expect("margins");
    assert_eq!(m[0].tier_no, 1);
    assert_eq!(m[0].buy_margin_pct.value(), d("0.6970"), "(33.23 ÷ 33 − 1) × 100, scale 4");
    assert_eq!(m[0].sell_margin_pct.value(), d("-0.3824"), "(33.87 ÷ 34 − 1) × 100, scale 4");
    assert!(!s.flags.contains(&PricingFlag::Precision));
    // Tier 4 sell: (33.80 ÷ 34 − 1) × 100 = −0.5882 → 34 × 0.994118 = 33.800012, which rounds half-up to 33.80.
    // Rounding it up (ceil) would give 33.81 and a false `Precision`.
    assert_eq!(m[3].sell_margin_pct.value(), d("-0.5882"));
}

#[test]
fn the_adjustment_is_taken_out_of_the_margin() {
    let mut i = usd();
    i.buy_adjust_pct = RatePercent::parse("0.1").unwrap();
    let m = PricingSuggestion::of(&i).margins.expect("margins");
    assert_eq!(m[0].buy_margin_pct.value(), d("0.5970"));
}

#[test]
fn no_base_means_no_margins_and_no_anchor() {
    let mut none = usd();
    none.base_buy = None;
    none.base_sell = None;
    let s = PricingSuggestion::of(&none);
    assert_eq!(s.margins, None);
    assert!(s.flags.contains(&PricingFlag::NoAnchor));
    let mut i = usd();
    i.base_sell = None;
    let s = PricingSuggestion::of(&i);
    assert_eq!(s.margins, None, "a missing side has no margin");
    assert!(s.flags.contains(&PricingFlag::NoAnchor));
    assert!(s.tiers.is_some(), "the prices are still suggested");
}
