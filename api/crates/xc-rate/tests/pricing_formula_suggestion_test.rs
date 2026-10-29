//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §3a; B21, B22 unit half, B23 margins, B26):
//! `PricingSuggestion::of_with` prices V.9's step 1 through the formula with the FOLLOW binding.
//!
//! Written before the implementation. Do not edit, delete or weaken. Pure: no database.

mod pricing_formula_shared;

use chrono::{TimeZone, Utc};
use pricing_formula_shared::*;
use rust_decimal::Decimal;
use xc_kernel::money::RatePercent;
use xc_rate::domain::formula::price_formula::PriceFormula;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_flag::PricingFlag;
use xc_rate::domain::pricing::pricing_suggestion::{
    PricingSuggestion, Suggestion, SuggestionConfig, SuggestionInput,
};

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

fn input(
    c: &str,
    quotes: Vec<MarketQuote>,
    tt_decimals: Option<u32>,
    unit_size: &str,
) -> SuggestionInput {
    SuggestionInput {
        currency: code(c),
        quotes,
        tt_decimals,
        unit_size: d(unit_size),
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

/// V.9's TT and Super Rich USD headlines with a BoT base of 33 / 34 (pricing_suggestion_test.rs `usd()`).
fn usd() -> SuggestionInput {
    let mut i = input(
        "USD",
        vec![
            quote(MarketSource::Tt, "33.2", "33.9"),
            quote(MarketSource::SuperRich, "33.33", "33.40"),
        ],
        Some(2),
        "1",
    );
    i.base_buy = Some(rate("33"));
    i.base_sell = Some(rate("34"));
    i
}

fn jpy() -> SuggestionInput {
    let mut i = input(
        "JPY",
        vec![quote(MarketSource::Tt, "0.2154", "0.2220")],
        None,
        "100",
    );
    i.base_buy = Some(rate("21.5"));
    i.base_sell = Some(rate("22.2"));
    i
}

fn prices(s: &Suggestion) -> Vec<(Decimal, Decimal)> {
    s.tiers
        .expect("tiers")
        .iter()
        .map(|t| (t.buy.expect("buy").value(), t.sell.expect("sell").value()))
        .collect()
}

#[test]
fn b21_the_default_formula_answers_exactly_what_of_answers() {
    let mut sr_only = usd();
    sr_only.quotes = vec![quote(MarketSource::SuperRich, "33.33", "33.40")];
    let mut no_base = usd();
    no_base.base_buy = None;
    no_base.base_sell = None;
    let mut narrow = usd();
    narrow.quotes = vec![quote(MarketSource::Tt, "33.50", "33.52")];
    let mut adjusted = usd();
    adjusted.buy_adjust_pct = pct("0.3000");
    adjusted.sell_adjust_pct = pct("-0.3000");
    for (name, i) in [
        ("usd", usd()),
        ("jpy at 100", jpy()),
        ("super rich only", sr_only),
        ("no base", no_base),
        ("min spread", narrow),
        ("policy percentages", adjusted),
        ("no anchor", input("CHF", vec![], None, "1")),
    ] {
        assert_eq!(
            PricingSuggestion::of_with(&i, &PriceFormula::default_formula()),
            PricingSuggestion::of(&i),
            "{name}"
        );
    }
}

#[test]
fn b22_an_organisation_formula_prices_step_1_then_v9s_steps_apply() {
    // TT 33.2 / 33.9, mid 33.55: ref_mid ± 0.05 gives 33.50 / 33.60 at every tier; the spread 0.10 is under the
    // 0.3 % minimum (0.10065), so V.9 widens to floor(33.499675) / ceil(33.600325) = 33.49 / 33.61
    let s = PricingSuggestion::of_with(&usd(), &mid_formula());
    assert_eq!(prices(&s), vec![(d("33.49"), d("33.61")); 4]);
    assert!(s.flags.contains(&PricingFlag::MinSpread), "{:?}", s.flags);
}

#[test]
fn b23_no_margins_under_a_non_default_formula() {
    let s = PricingSuggestion::of_with(&usd(), &mid_formula());
    assert!(s.tiers.is_some());
    assert_eq!(
        s.margins, None,
        "no inverse of a formula that is not the default"
    );
    let default = PricingSuggestion::of_with(&usd(), &PriceFormula::default_formula());
    assert!(
        default.margins.is_some(),
        "the default keeps today's margins"
    );
}

#[test]
fn b26_follow_constants_are_per_the_currencys_unit() {
    // JPY TT 0.2154 / 0.2220 per 1 JPY; at the unit size 100: 21.54 / 22.20, mid 21.87 ⇒ 21.82 / 21.92 per 100 JPY
    // (0.2182 / 0.2192 per 1 JPY, 4 places), not 0.05 per 1 JPY
    let s = PricingSuggestion::of_with(&jpy(), &mid_formula());
    assert_eq!(prices(&s)[0], (d("21.82"), d("21.92")));
}

#[test]
fn the_follow_binding_is_zero_policy_percentages_and_plus_minus_better() {
    // buy_pct / sell_pct are 0 under FOLLOW even when the policy has percentages; tier n gets +better[n] / −better[n]
    let f = formula(
        "ref_buy * (1 + (buy_pct + tier_buy_pct) / 100)",
        "ref_sell * (1 + (sell_pct + tier_sell_pct) / 100)",
    );
    let mut adjusted = usd();
    adjusted.buy_adjust_pct = pct("0.3000");
    adjusted.sell_adjust_pct = pct("-0.3000");
    assert_eq!(
        prices(&PricingSuggestion::of_with(&adjusted, &f)),
        prices(&PricingSuggestion::of(&usd())),
        "the policy's percentages do not move FOLLOW prices"
    );
}

#[test]
fn a_formula_error_gives_no_tiers_and_no_anchor() {
    let s = PricingSuggestion::of_with(
        &usd(),
        &formula("ref_buy / (buy_pct - buy_pct)", "ref_sell"),
    );
    assert_eq!(s.tiers, None);
    assert_eq!(s.margins, None);
    assert!(s.flags.contains(&PricingFlag::NoAnchor), "{:?}", s.flags);
}
