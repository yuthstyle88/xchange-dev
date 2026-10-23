//! XC-PRICING PX-2 (pricing-tiers.md §4): `TtFeed::parse`, ported from V.9 `test/pricing.mjs:33-40,48-51,57-58`
//! (`parseTT`). Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit,
//! delete or weaken; add tests in a file of your own. Pure: no network.

use rust_decimal::Decimal;
use serde_json::json;
use xc_rate::domain::pricing::market_feed::{MarketBody, MarketFeed};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::market_snapshot::MarketRow;
use xc_rate::infrastructure::market::tt_feed::{TT_RATES_URL, TT_UPDATED_URL, TtFeed};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

/// V.9 `test/pricing.mjs:33-40`, shaped like the live response of 2026-10-18.
fn fixture() -> String {
    json!([
        {"name": "USD(L)", "description": "100-50", "current_buy_rate": 33.2, "current_sell_rate": 33.9, "decimals": 2},
        {"name": "USD(S)", "description": "2-1", "current_buy_rate": 32.3, "current_sell_rate": 33.9, "decimals": 2},
        {"name": "JPY", "description": "10000-1000", "current_buy_rate": 0.2085, "current_sell_rate": 0.2175, "decimals": 4},
        {"name": "EUR(L)", "description": "500", "current_buy_rate": 37.25, "current_sell_rate": 38, "decimals": 2},
        {"name": "XXX", "description": "", "current_buy_rate": 0, "current_sell_rate": 0},
        {"name": "not a code", "current_buy_rate": 1, "current_sell_rate": 2}
    ])
    .to_string()
}

fn bodies(rates: &str, updated: Option<&str>) -> Vec<MarketBody> {
    vec![
        MarketBody {
            url: TT_RATES_URL.to_string(),
            body: Some(rates.to_string()),
        },
        MarketBody {
            url: TT_UPDATED_URL.to_string(),
            body: updated.map(str::to_string),
        },
    ]
}

#[test]
fn live_feed_asks_for_the_rates_and_optionally_the_update_time() {
    let feed = TtFeed::live();
    assert_eq!(feed.source(), MarketSource::Tt);
    let req = feed.requests();
    assert_eq!(req.len(), 2);
    assert_eq!(
        req[0].url,
        "https://api.software.ttexchange.com/currencies?is_main=false&branch_id=1"
    );
    assert!(!req[0].optional);
    assert_eq!(
        req[1].url,
        "https://api.software.ttexchange.com/currencies/updated_at?branch_id=1"
    );
    assert!(req[1].optional);
}

#[test]
fn parses_the_fixture_into_four_rows_with_bands() {
    let snap = TtFeed::live()
        .parse(&bodies(
            &fixture(),
            Some(r#"{"updated_at":"2026-10-17T18:17:40+07:00"}"#),
        ))
        .expect("parsed");
    assert_eq!(snap.source, MarketSource::Tt);
    assert_eq!(
        snap.source_updated_at.as_deref(),
        Some("2026-10-17T18:17:40+07:00")
    );
    assert_eq!(
        snap.rows.len(),
        4,
        "XXX (no price) and 'not a code' are skipped"
    );
    assert!(snap.rows.contains(&MarketRow {
        code: "USD".into(),
        band: "L".into(),
        denoms: "100-50".into(),
        buy: Some(d("33.2")),
        sell: Some(d("33.9")),
        decimals: Some(2),
    }));
    let jpy = snap.rows.iter().find(|r| r.code == "JPY").unwrap();
    assert_eq!(
        (jpy.band.as_str(), jpy.buy, jpy.decimals),
        ("", Some(d("0.2085")), Some(4))
    );
}

#[test]
fn a_missing_update_time_does_not_fail_tt() {
    let snap = TtFeed::live()
        .parse(&bodies(&fixture(), None))
        .expect("parsed");
    assert_eq!(snap.source_updated_at, None);
    assert_eq!(snap.rows.len(), 4);
}

#[test]
fn a_body_that_is_not_an_array_is_unexpected() {
    let err = TtFeed::live()
        .parse(&bodies(r#"{"error":"x"}"#, None))
        .unwrap_err();
    assert!(err.to_string().contains("TT: unexpected response"), "{err}");
}

#[test]
fn an_empty_array_has_no_rates() {
    let err = TtFeed::live().parse(&bodies("[]", None)).unwrap_err();
    assert!(err.to_string().contains("TT: no rates"), "{err}");
}
