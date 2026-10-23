//! XC-PRICING PX-8 (pricing-tiers.md §5.2): `Recommendation::list`, ported from V.9 `test/pricing.mjs:109-127`
//! (`recommend`). Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit,
//! delete or weaken; add tests in a file of your own. Pure.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::Rate;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::recommendation::{
    CounterRate, Recommendation, RecommendationCode as C, RecommendationInput, RecommendationSide,
    Severity,
};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn tt(buy: &str, sell: &str) -> MarketQuote {
    MarketQuote {
        source: MarketSource::Tt,
        buy: Some(Rate::parse(buy).unwrap()),
        sell: Some(Rate::parse(sell).unwrap()),
        unit_size: Decimal::ONE,
        fetched_at: Utc.with_ymd_and_hms(2026, 10, 18, 3, 2, 0).unwrap(),
        stale: false,
    }
}

fn counter(
    code: &str,
    buy: &str,
    sell: &str,
    cost: Option<&str>,
    published: &str,
    anchor: MarketQuote,
) -> CounterRate {
    CounterRate {
        currency: code.to_string(),
        buy: d(buy),
        sell: d(sell),
        partner_cost: cost.map(d),
        published_at: Some(at(published)),
        anchor: Some(anchor),
    }
}

/// V.9 `test/pricing.mjs:110-116`: TT USD 33.2/33.9, EUR 37.25/38, JPY 0.2085/0.2175; TT updated
/// 2026-10-17T18:17:40+07:00. Volumes: EUR 1000 × 37.5, USD 100 × 34 (the 99,999 USD order is older than 7 days and
/// is not in the map the caller builds).
fn fixture() -> RecommendationInput {
    RecommendationInput {
        tt_stale: false,
        sr_stale: false,
        tt_source_updated_at: Some(at("2026-10-17T18:17:40+07:00")),
        counters: vec![
            counter(
                "USD",
                "32.8",
                "34.4",
                None,
                "2026-10-17T09:00:00Z",
                tt("33.2", "33.9"),
            ),
            counter(
                "EUR",
                "37.2",
                "37.5",
                None,
                "2026-10-18T04:00:00Z",
                tt("37.25", "38"),
            ),
            counter(
                "JPY",
                "0.2085",
                "0.2175",
                Some("0.22"),
                "2026-10-18T04:00:00Z",
                tt("0.2085", "0.2175"),
            ),
        ],
        listed_by_both: vec!["USD".into()],
        enabled: HashSet::from(["USD".to_string(), "EUR".to_string(), "JPY".to_string()]),
        volume_thb: HashMap::from([
            ("EUR".to_string(), d("37500")),
            ("USD".to_string(), d("3400")),
        ]),
    }
}

fn find(list: &[Recommendation], code: C, currency: &str) -> Option<Recommendation> {
    list.iter()
        .find(|r| r.code == code && r.currency.as_deref() == Some(currency))
        .cloned()
}

#[test]
fn v9_fixture_gives_v9s_recommendations() {
    let recs = Recommendation::list(&fixture());
    let sell = find(&recs, C::SellAboveMarket, "USD").expect("USD sell above TT");
    assert_eq!(
        (sell.severity, sell.side, sell.gap_pct),
        (
            Severity::High,
            Some(RecommendationSide::Sell),
            Some(d("1.47"))
        )
    );
    assert_eq!(sell.value, Some(d("33.9")));
    let buy = find(&recs, C::BuyBelowMarket, "USD").expect("USD buy below TT");
    assert_eq!(
        (buy.severity, buy.gap_pct),
        (Severity::High, Some(d("-1.20")))
    );
    assert_eq!(
        find(&recs, C::SellBelowMarket, "EUR").map(|r| r.severity),
        Some(Severity::Medium)
    );
    assert_eq!(
        find(&recs, C::BelowPartnerCost, "JPY").map(|r| r.severity),
        Some(Severity::High)
    );
    assert_eq!(
        find(&recs, C::MarketMovedSinceUpdate, "USD").map(|r| r.severity),
        Some(Severity::Low)
    );
    assert_eq!(
        find(&recs, C::SpreadWide, "USD").map(|r| r.severity),
        Some(Severity::Medium)
    );
    assert!(
        find(&recs, C::MarketMovedSinceUpdate, "EUR").is_none(),
        "EUR was published after TT's update"
    );
    assert!(
        find(&recs, C::BuyBelowMarket, "EUR").is_none(),
        "37.2 vs 37.25 is within 1 %"
    );
}

#[test]
fn ordered_by_severity_then_volume() {
    let recs = Recommendation::list(&fixture());
    assert_eq!(recs[0].severity, Severity::High);
    assert_eq!(
        find(&recs, C::SellAboveMarket, "USD").unwrap().volume_thb,
        d("3400"),
        "only the last 7 days count"
    );
    let jpy = recs
        .iter()
        .position(|r| r.currency.as_deref() == Some("JPY"))
        .unwrap();
    let medium = recs
        .iter()
        .position(|r| r.severity == Severity::Medium)
        .unwrap();
    assert!(jpy < medium, "JPY's High comes before every Medium");
    for w in recs.windows(2) {
        assert!(w[0].severity <= w[1].severity, "{:?}", recs);
        if w[0].severity == w[1].severity {
            assert!(w[0].volume_thb >= w[1].volume_thb, "{:?}", recs);
        }
    }
}

#[test]
fn a_stale_source_comes_first() {
    let mut i = fixture();
    i.tt_stale = true;
    let recs = Recommendation::list(&i);
    // High rows sort before Warn, so the first Warn row is the staleness one, with no currency (V.9 test :126-127
    // checks it is the first row when nothing is High).
    let warn = recs
        .iter()
        .find(|r| r.severity == Severity::Warn)
        .expect("a warn row");
    assert_eq!((warn.code, warn.currency.clone()), (C::TtStale, None));
    let only = Recommendation::list(&RecommendationInput {
        tt_stale: true,
        ..Default::default()
    });
    assert_eq!(only[0].code, C::TtStale);
}

#[test]
fn a_currency_both_sources_list_but_the_branch_does_not_sell_is_not_offered() {
    let mut i = fixture();
    i.listed_by_both = vec!["USD".into(), "GBP".into(), "CHF".into()];
    let recs = Recommendation::list(&i);
    let not_offered: Vec<_> = recs.iter().filter(|r| r.code == C::NotOffered).collect();
    assert_eq!(not_offered.len(), 2, "one row per currency, USD is sold");
    assert!(not_offered.iter().all(|r| r.severity == Severity::Low));
}

#[test]
fn a_counter_without_an_anchor_gives_no_market_rows() {
    let mut i = fixture();
    i.counters[0].anchor = None;
    let recs = Recommendation::list(&i);
    assert!(recs.iter().all(|r| r.currency.as_deref() != Some("USD")));
}
