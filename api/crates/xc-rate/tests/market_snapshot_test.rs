//! XC-PRICING PX-1 (pricing-tiers.md §4): `MarketSnapshot::headline` and `MarketStaleness::is_stale`, ported from V.9
//! `test/pricing.mjs:48-55,70` (`headline`, stale after 30 minutes). Written before the implementation by the ticket
//! author (claude/mbp-xc-pricing, PX-0). Do not edit, delete or weaken; add tests in a file of your own. Pure.

use chrono::{Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::market_snapshot::{MarketRow, MarketSnapshot, MarketStaleness};

fn d(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn row(
    code: &str,
    band: &str,
    denoms: &str,
    buy: &str,
    sell: &str,
    decimals: Option<u32>,
) -> MarketRow {
    MarketRow {
        code: code.to_string(),
        band: band.to_string(),
        denoms: denoms.to_string(),
        buy: Some(d(buy)),
        sell: Some(d(sell)),
        decimals,
    }
}

#[test]
fn tt_headline_takes_band_l_then_no_band_then_m_then_s() {
    let snap = MarketSnapshot {
        source: MarketSource::Tt,
        source_updated_at: None,
        rows: vec![
            row("USD", "S", "2-1", "32.3", "33.9", Some(2)),
            row("USD", "L", "100-50", "33.2", "33.9", Some(2)),
            row("USD", "M", "20-10", "33.0", "33.9", Some(2)),
            row("JPY", "", "10000-1000", "0.2085", "0.2175", Some(4)),
            row("EUR", "M", "50", "37.0", "38", Some(2)),
            row("EUR", "", "500", "37.25", "38", Some(2)),
        ],
    };
    let h = snap.headline();
    assert_eq!(
        h.iter().map(|r| r.code.as_str()).collect::<Vec<_>>(),
        ["EUR", "JPY", "USD"],
        "one row per currency, sorted by code"
    );
    let usd = h.iter().find(|r| r.code == "USD").unwrap();
    assert_eq!(usd, &row("USD", "L", "100-50", "33.2", "33.9", Some(2)));
    assert_eq!(h.iter().find(|r| r.code == "JPY").unwrap().band, "");
    assert_eq!(
        h.iter().find(|r| r.code == "EUR").unwrap().band,
        "",
        "no band ranks before M"
    );
}

#[test]
fn super_rich_headline_takes_the_first_list_entry() {
    let snap = MarketSnapshot {
        source: MarketSource::SuperRich,
        source_updated_at: Some("2026-10-18".to_string()),
        rows: vec![
            row("USD", "1", "5-1", "32.10", "33.40", None),
            row("USD", "0", "100", "33.33", "33.40", None),
            row("GBP", "0", "50", "44.40", "44.55", None),
        ],
    };
    let h = snap.headline();
    let usd = h.iter().find(|r| r.code == "USD").unwrap();
    assert_eq!(
        (usd.buy, usd.sell, usd.denoms.as_str()),
        (Some(d("33.33")), Some(d("33.40")), "100")
    );
    assert_eq!(h.len(), 2);
}

#[test]
fn stale_with_no_snapshot_or_after_thirty_minutes() {
    let now = Utc.with_ymd_and_hms(2026, 10, 18, 3, 31, 0).unwrap();
    assert!(
        MarketStaleness::is_stale(None, now),
        "no good snapshot is stale"
    );
    assert!(!MarketStaleness::is_stale(
        Some(now - Duration::minutes(6)),
        now
    ));
    assert!(
        !MarketStaleness::is_stale(Some(now - Duration::minutes(30)), now),
        "30 minutes exactly is not yet stale"
    );
    assert!(MarketStaleness::is_stale(
        Some(now - Duration::minutes(31)),
        now
    ));
}
