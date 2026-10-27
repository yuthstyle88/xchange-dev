//! XC-BE-GAPS-A row 1: the shortfall a refused posting carries (`xc-cash/src/domain/shortfall.rs`; design
//! `docs/design/blueprint/ledger-inventory.md:418-425` §5.1, "requested by the caller that got `stockInsufficient`
//! … with the shortfall per currency in `proposed_payload`").
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 1, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Pinned here (pure, no database):
//! - `ShortPlace::as_str` is `DRAWER`, `SAFE`, `POSITION`;
//! - `StockShortfall::refusal()` is `AppError::Domain` with the §4.5 sentence the port already sends
//!   (`ledger_postings.rs` `stock_insufficient(..)`), then `MARKER`, then a compact JSON array of
//!   `{"currency","place","shortBy"}` in `lines` order, `shortBy` the normalized decimal as a string — so every
//!   reader that matches `starts_with("stockInsufficient")` / `contains("stockInsufficient")` still matches;
//! - `from_refusal` reads back what `refusal` wrote, and answers an empty shortfall (never a panic) for a message
//!   with no marker, an array it cannot read, or a place it does not know;
//! - `payload()` is `{"shortfall": [..]}`; empty is today's `{"shortfall": []}` (`negative_stock_gate.rs`).

use std::str::FromStr;

use pos_error::AppError;
use rust_decimal::Decimal;
use serde_json::json;
use xc_cash::domain::shortfall::{ShortPlace, Shortfall, StockShortfall};
use xc_kernel::money::CurrencyCode;

const SENTENCE: &str =
    "stockInsufficient: the posting takes a drawer, a safe or a position below zero";

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn line(code: &str, place: ShortPlace, short_by: &str) -> Shortfall {
    Shortfall {
        currency: CurrencyCode::parse(code).expect("a currency"),
        place,
        short_by: d(short_by),
    }
}

fn message_of(e: AppError) -> String {
    match e {
        AppError::Domain(m) => m,
        other => panic!("refusal() is AppError::Domain, got {other:?}"),
    }
}

/// Lines with trailing zeros, two currencies and every place, in an order that is not sorted: `refusal` keeps
/// `lines` order (the port sorts, this type does not).
fn three_lines() -> StockShortfall {
    StockShortfall {
        lines: vec![
            line("USD", ShortPlace::Drawer, "1000.00"),
            line("EUR", ShortPlace::Safe, "12.50"),
            line("USD", ShortPlace::Position, "0.5"),
        ],
    }
}

#[test]
fn gapsa1_short_place_as_str() {
    assert_eq!(ShortPlace::Drawer.as_str(), "DRAWER");
    assert_eq!(ShortPlace::Safe.as_str(), "SAFE");
    assert_eq!(ShortPlace::Position.as_str(), "POSITION");
}

#[test]
fn gapsa1_refusal_is_the_sentence_then_the_marker_then_a_compact_array_in_lines_order() {
    let m = message_of(three_lines().refusal());
    assert_eq!(
        m,
        format!(
            "{SENTENCE}; shortfall=[\
             {{\"currency\":\"USD\",\"place\":\"DRAWER\",\"shortBy\":\"1000\"}},\
             {{\"currency\":\"EUR\",\"place\":\"SAFE\",\"shortBy\":\"12.5\"}},\
             {{\"currency\":\"USD\",\"place\":\"POSITION\",\"shortBy\":\"0.5\"}}]"
        ),
        "the §4.5 sentence, the marker, then each line as {{currency, place, shortBy}} with shortBy normalized"
    );
    assert_eq!(StockShortfall::MARKER, "; shortfall=");
    assert!(
        m.starts_with("stockInsufficient"),
        "readers that match the code prefix still match"
    );
}

#[test]
fn gapsa1_refusal_of_no_lines_carries_an_empty_array() {
    let m = message_of(StockShortfall::default().refusal());
    assert_eq!(m, format!("{SENTENCE}; shortfall=[]"));
}

#[test]
fn gapsa1_from_refusal_round_trips_the_lines() {
    let lines = three_lines();
    let m = message_of(lines.refusal());
    let back = StockShortfall::from_refusal(&m);
    assert_eq!(back, lines, "what refusal() wrote reads back equal");
    // Decimal equality ignores scale, so check the normalized figures too.
    let figures: Vec<String> = back.lines.iter().map(|l| l.short_by.to_string()).collect();
    assert_eq!(figures, vec!["1000", "12.5", "0.5"], "read back normalized");
    let codes: Vec<&str> = back.lines.iter().map(|l| l.currency.as_str()).collect();
    assert_eq!(codes, vec!["USD", "EUR", "USD"], "in the order written");
}

#[test]
fn gapsa1_from_refusal_of_an_empty_array_is_empty() {
    let m = message_of(StockShortfall::default().refusal());
    assert_eq!(StockShortfall::from_refusal(&m), StockShortfall::default());
}

#[test]
fn gapsa1_from_refusal_without_the_marker_is_empty() {
    assert_eq!(
        StockShortfall::from_refusal("stockInsufficient: x"),
        StockShortfall::default()
    );
    assert_eq!(
        StockShortfall::from_refusal(SENTENCE),
        StockShortfall::default(),
        "today's refusal text carries no shortfall"
    );
    assert_eq!(StockShortfall::from_refusal(""), StockShortfall::default());
}

#[test]
fn gapsa1_from_refusal_of_an_unreadable_array_is_empty() {
    for tail in [
        "not json",
        "[",
        "{\"currency\":\"USD\",\"place\":\"DRAWER\",\"shortBy\":\"1\"}",
        "[{\"currency\":\"USD\",\"place\":\"DRAWER\"}]",
        "[{\"currency\":\"USD\",\"place\":\"DRAWER\",\"shortBy\":\"lots\"}]",
        "[{\"currency\":\"XYZ\",\"place\":\"DRAWER\",\"shortBy\":\"1\"}]",
    ] {
        let m = format!("{SENTENCE}; shortfall={tail}");
        assert_eq!(
            StockShortfall::from_refusal(&m),
            StockShortfall::default(),
            "unreadable shortfall {tail:?} reads as empty"
        );
    }
}

#[test]
fn gapsa1_from_refusal_of_an_unknown_place_is_empty() {
    let m = format!(
        "{SENTENCE}; shortfall=[{{\"currency\":\"USD\",\"place\":\"DRAWER\",\"shortBy\":\"1\"}},\
         {{\"currency\":\"USD\",\"place\":\"VAULT\",\"shortBy\":\"2\"}}]"
    );
    assert_eq!(StockShortfall::from_refusal(&m), StockShortfall::default());
}

#[test]
fn gapsa1_payload_carries_the_same_objects() {
    assert_eq!(
        three_lines().payload(),
        json!({
            "shortfall": [
                {"currency": "USD", "place": "DRAWER", "shortBy": "1000"},
                {"currency": "EUR", "place": "SAFE", "shortBy": "12.5"},
                {"currency": "USD", "place": "POSITION", "shortBy": "0.5"},
            ]
        })
    );
}

#[test]
fn gapsa1_payload_of_no_lines_is_todays_shape() {
    assert_eq!(
        StockShortfall::default().payload(),
        json!({ "shortfall": [] })
    );
}

#[test]
fn gapsa1_payload_of_what_a_refusal_carries() {
    let m = message_of(three_lines().refusal());
    assert_eq!(
        StockShortfall::from_refusal(&m).payload(),
        three_lines().payload()
    );
}
