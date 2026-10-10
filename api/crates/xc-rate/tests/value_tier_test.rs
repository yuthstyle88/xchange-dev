//! Rate feed F2 — value tiers (`docs/design/rate-feed-connector.md` §2, §3.4, §3.8).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! What this pins, in `api/crates/xc-rate/src/domain/publication/value_tier.rs`:
//! - `TierCode::new(text) -> Result<TierCode, String>`: refuses an empty or blank code, keeps the
//!   text as given otherwise; `as_str()` answers it.
//! - `TierBand { code, lower_exclusive_thb: Option<Money>, upper_inclusive_thb: Option<Money> }`:
//!   `contains(v)` is `lower < v ≤ upper`, a missing bound being open (BP §02: "≤ 10,000",
//!   "> 10,000 – 50,000", "> 200,000").
//! - `TierTable::new(bands)`, `pick(base_value_thb) -> Option<TierCode>`: the code of the one band
//!   that contains the value; `None` when no band does (a gap) or more than one does (an overlap),
//!   never a guess.

use xc_kernel::money::Money;
use xc_rate::domain::publication::value_tier::{TierBand, TierCode, TierTable};

fn thb(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text} is a THB amount: {e:?}"))
}

fn code(text: &str) -> TierCode {
    TierCode::new(text).unwrap_or_else(|e| panic!("{text} is a tier code: {e}"))
}

fn band(c: &str, lower: Option<&str>, upper: Option<&str>) -> TierBand {
    TierBand {
        code: code(c),
        lower_exclusive_thb: lower.map(thb),
        upper_inclusive_thb: upper.map(thb),
    }
}

/// BP §02's four tiers.
fn blueprint_tiers() -> TierTable {
    TierTable::new(vec![
        band("RETAIL", None, Some("10000.00")),
        band("STANDARD", Some("10000.00"), Some("50000.00")),
        band("LARGE", Some("50000.00"), Some("200000.00")),
        band("SPECIAL", Some("200000.00"), None),
    ])
}

fn picked(table: &TierTable, value: &str) -> Option<String> {
    table.pick(thb(value)).map(|c| c.as_str().to_owned())
}

#[test]
fn a_tier_code_refuses_blank_text_and_keeps_the_rest_as_given() {
    assert!(TierCode::new("").is_err(), "empty");
    assert!(TierCode::new("   ").is_err(), "blank");
    assert_eq!(code("STANDARD").as_str(), "STANDARD");
    assert_eq!(code("retail-1").as_str(), "retail-1");
}

#[test]
fn a_band_is_lower_exclusive_and_upper_inclusive() {
    let standard = band("STANDARD", Some("10000.00"), Some("50000.00"));
    assert!(
        !standard.contains(thb("10000.00")),
        "the lower edge belongs to the band below"
    );
    assert!(standard.contains(thb("10000.01")));
    assert!(
        standard.contains(thb("50000.00")),
        "the upper edge belongs to this band"
    );
    assert!(!standard.contains(thb("50000.01")));
}

#[test]
fn a_missing_bound_is_open() {
    let retail = band("RETAIL", None, Some("10000.00"));
    assert!(retail.contains(thb("0.00")));
    assert!(retail.contains(thb("0.01")));
    let special = band("SPECIAL", Some("200000.00"), None);
    assert!(special.contains(thb("99999999.99")));
    assert!(!special.contains(thb("200000.00")));
}

#[test]
fn the_blueprint_edges_pick_the_lower_tier() {
    let t = blueprint_tiers();
    assert_eq!(picked(&t, "0.01").as_deref(), Some("RETAIL"));
    assert_eq!(
        picked(&t, "10000.00").as_deref(),
        Some("RETAIL"),
        "BP §02: ≤ 10,000"
    );
    assert_eq!(picked(&t, "10000.01").as_deref(), Some("STANDARD"));
    assert_eq!(picked(&t, "50000.00").as_deref(), Some("STANDARD"));
    assert_eq!(picked(&t, "50000.01").as_deref(), Some("LARGE"));
    assert_eq!(picked(&t, "200000.00").as_deref(), Some("LARGE"));
    assert_eq!(picked(&t, "200000.01").as_deref(), Some("SPECIAL"));
}

#[test]
fn a_gap_picks_nothing() {
    let t = TierTable::new(vec![
        band("RETAIL", None, Some("10000.00")),
        band("LARGE", Some("50000.00"), None),
    ]);
    assert_eq!(
        picked(&t, "20000.00"),
        None,
        "no band holds 20,000: unavailable, not a guess"
    );
    assert_eq!(picked(&t, "9000.00").as_deref(), Some("RETAIL"));
}

#[test]
fn an_overlap_picks_nothing() {
    let t = TierTable::new(vec![
        band("A", None, Some("20000.00")),
        band("B", Some("10000.00"), None),
    ]);
    assert_eq!(picked(&t, "15000.00"), None, "two bands hold 15,000");
    assert_eq!(picked(&t, "5000.00").as_deref(), Some("A"));
    assert_eq!(picked(&t, "25000.00").as_deref(), Some("B"));
}

#[test]
fn an_empty_table_picks_nothing() {
    assert_eq!(picked(&TierTable::new(vec![]), "100.00"), None);
}

#[test]
fn the_order_of_the_bands_does_not_matter() {
    let t = TierTable::new(vec![
        band("SPECIAL", Some("200000.00"), None),
        band("RETAIL", None, Some("10000.00")),
        band("LARGE", Some("50000.00"), Some("200000.00")),
        band("STANDARD", Some("10000.00"), Some("50000.00")),
    ]);
    assert_eq!(picked(&t, "10000.00").as_deref(), Some("RETAIL"));
    assert_eq!(picked(&t, "123456.78").as_deref(), Some("LARGE"));
}
