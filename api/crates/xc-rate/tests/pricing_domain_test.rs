//! PT-8 (pricing-tiers.md §5.1, §5.2, §11): `MarketQuote::anchor_for`, `PricingFlag::blocking`, and the one
//! `RowStatus::of` row §11 fixes (no market quote ⇒ `Manual`). The other statuses wait for V.9's files (P-2).
//! Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not edit, delete or weaken;
//! add tests in a file of your own. Pure: no database, no clock.

use std::collections::HashSet;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use xc_kernel::money::Rate;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_flag::PricingFlag;
use xc_rate::domain::pricing::pricing_suggestion::Suggestion;
use xc_rate::domain::pricing::row_status::RowStatus;

fn quote(source: MarketSource, buy: &str, sell: &str, stale: bool) -> MarketQuote {
    MarketQuote {
        source,
        buy: Some(Rate::parse(buy).expect("rate")),
        sell: Some(Rate::parse(sell).expect("rate")),
        unit_size: Decimal::ONE,
        fetched_at: Utc.with_ymd_and_hms(2026, 10, 20, 1, 0, 0).unwrap(),
        stale,
    }
}

fn flags(list: &[PricingFlag]) -> HashSet<PricingFlag> {
    list.iter().copied().collect()
}

#[test]
fn tt_present_anchors_on_tt_with_no_flag() {
    let quotes = vec![
        quote(MarketSource::SuperRich, "35.10", "35.30", false),
        quote(MarketSource::Tt, "35.00", "35.40", false),
        quote(MarketSource::Bot, "34.90", "35.50", false),
    ];
    let (anchor, fl) = MarketQuote::anchor_for(&quotes);
    assert_eq!(anchor.map(|q| q.source), Some(MarketSource::Tt));
    assert_eq!(anchor, Some(&quotes[1]));
    assert!(fl.is_empty(), "no flag when TT is there and fresh: {fl:?}");
}

#[test]
fn no_tt_quote_anchors_on_super_rich_and_flags_no_tt_quote() {
    let quotes = vec![
        quote(MarketSource::Bot, "34.90", "35.50", false),
        quote(MarketSource::SuperRich, "35.10", "35.30", false),
    ];
    let (anchor, fl) = MarketQuote::anchor_for(&quotes);
    assert_eq!(anchor, Some(&quotes[1]));
    assert_eq!(flags(&fl), flags(&[PricingFlag::NoTtQuote]));
}

#[test]
fn no_market_quote_has_no_anchor_and_flags_no_anchor() {
    let (anchor, fl) = MarketQuote::anchor_for(&[]);
    assert_eq!(anchor, None);
    assert_eq!(flags(&fl), flags(&[PricingFlag::NoAnchor]));
}

#[test]
fn a_bot_quote_alone_is_never_a_market_anchor() {
    let quotes = vec![quote(MarketSource::Bot, "34.90", "35.50", false)];
    let (anchor, fl) = MarketQuote::anchor_for(&quotes);
    assert_eq!(anchor, None);
    assert_eq!(flags(&fl), flags(&[PricingFlag::NoAnchor]));
}

#[test]
fn a_stale_tt_anchor_flags_stale_source() {
    let quotes = vec![
        quote(MarketSource::Tt, "35.00", "35.40", true),
        quote(MarketSource::SuperRich, "35.10", "35.30", false),
    ];
    let (anchor, fl) = MarketQuote::anchor_for(&quotes);
    assert_eq!(
        anchor,
        Some(&quotes[0]),
        "TT is still the anchor when stale"
    );
    assert_eq!(flags(&fl), flags(&[PricingFlag::StaleSource]));
}

#[test]
fn a_stale_super_rich_anchor_flags_both() {
    let quotes = vec![quote(MarketSource::SuperRich, "35.10", "35.30", true)];
    let (anchor, fl) = MarketQuote::anchor_for(&quotes);
    assert_eq!(anchor, Some(&quotes[0]));
    assert_eq!(
        flags(&fl),
        flags(&[PricingFlag::NoTtQuote, PricingFlag::StaleSource])
    );
}

#[test]
fn a_stale_quote_that_is_not_the_anchor_adds_no_flag() {
    let quotes = vec![
        quote(MarketSource::Tt, "35.00", "35.40", false),
        quote(MarketSource::SuperRich, "35.10", "35.30", true),
    ];
    let (_, fl) = MarketQuote::anchor_for(&quotes);
    assert!(fl.is_empty(), "only the anchor's staleness counts: {fl:?}");
}

#[test]
fn blocking_flags_keep_a_row_out_of_bulk_approve() {
    for f in [
        PricingFlag::NoAnchor,
        PricingFlag::StaleSource,
        PricingFlag::BelowCost,
        PricingFlag::Precision,
    ] {
        assert!(f.blocking(), "{f:?} blocks (§5.1, §11)");
    }
    for f in [
        PricingFlag::NoTtQuote,
        PricingFlag::MinSpread,
        PricingFlag::LargeMove,
    ] {
        assert!(
            !f.blocking(),
            "{f:?} warns only (§11: LargeMove warns, does not block)"
        );
    }
}

#[test]
fn a_suggestion_is_blocked_when_any_flag_blocks() {
    let s = Suggestion {
        tiers: None,
        margins: None,
        flags: vec![PricingFlag::LargeMove, PricingFlag::BelowCost],
    };
    assert!(s.blocked());
    let s = Suggestion {
        tiers: None,
        margins: None,
        flags: vec![PricingFlag::LargeMove, PricingFlag::NoTtQuote],
    };
    assert!(!s.blocked());
}

#[test]
fn no_market_quote_is_status_manual() {
    let s = Suggestion {
        tiers: None,
        margins: None,
        flags: vec![PricingFlag::NoAnchor],
    };
    assert_eq!(RowStatus::of(&s, false, true), RowStatus::Manual);
    assert_eq!(RowStatus::of(&s, true, true), RowStatus::Manual);
}
