//! RF-1 (design `docs/design/blueprint/rates.md` §4.10, PR 843): the one freshness rule. A side is stale when its
//! reference snapshot has `fetched_at` and `max_reference_age_hours`, and `now − fetched_at > max_reference_age_hours`;
//! a side with no reference is never stale; a publication is stale when any side is, and its `source_date` is the
//! oldest one. Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Pure: no database. The time is passed in, never read from the clock.
//!
//! Signatures these tests assume (§4.10 [required]):
//! - `xc_rate::domain::publication::freshness::Freshness { pub stale: bool, pub source_date: Option<NaiveDate> }`,
//!   `Clone + Debug + PartialEq + Eq`.
//! - `Freshness::of_side(snapshot: &serde_json::Value, side: Side, now: DateTime<Utc>) -> Freshness`.
//! - `Freshness::of_publication(sides: impl IntoIterator<Item = Freshness>) -> Freshness`.
//! - `Side` is `xc_kernel::money::Side`.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};
use xc_kernel::money::Side;
use xc_rate::domain::publication::freshness::Freshness;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 12, 0, 0).unwrap()
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

/// A line's `source_snapshot` as the publish command writes it (`rates_re4_bookings_test.rs:342-350`).
fn snapshot(
    buy_mode: &str,
    sell_mode: &str,
    fetched_hours_ago: Option<i64>,
    max_age: Option<i64>,
) -> Value {
    let mut s = json!({ "currency": "USD", "buy_mode": buy_mode, "sell_mode": sell_mode });
    if let Some(h) = fetched_hours_ago {
        let at = now() - Duration::hours(h);
        s["fetched_at"] = json!(at.to_rfc3339());
        s["source_date"] = json!(at.format("%Y-%m-%d").to_string());
    }
    if let Some(m) = max_age {
        s["max_reference_age_hours"] = json!(m);
    }
    s
}

fn fresh(source_date: Option<&str>) -> Freshness {
    Freshness {
        stale: false,
        source_date: source_date.map(date),
    }
}

fn stale(source_date: Option<&str>) -> Freshness {
    Freshness {
        stale: true,
        source_date: source_date.map(date),
    }
}

// ── one side ──────────────────────────────────────────────────────────────

#[test]
fn an_auto_side_past_its_age_is_stale() {
    let s = snapshot("AUTO", "AUTO", Some(30), Some(24));
    assert_eq!(
        Freshness::of_side(&s, Side::Buy, now()),
        stale(Some("2026-10-09"))
    );
    assert_eq!(
        Freshness::of_side(&s, Side::Sell, now()),
        stale(Some("2026-10-09"))
    );
}

#[test]
fn an_auto_side_within_its_age_is_fresh() {
    let s = snapshot("AUTO", "AUTO", Some(1), Some(24));
    assert_eq!(
        Freshness::of_side(&s, Side::Buy, now()),
        fresh(Some("2026-10-10"))
    );
}

#[test]
fn exactly_at_the_age_is_still_fresh() {
    let s = snapshot("AUTO", "AUTO", Some(24), Some(24));
    assert!(
        !Freshness::of_side(&s, Side::Buy, now()).stale,
        "stale only when older than the limit"
    );
}

/// The case the two views get wrong on `fcc1247` (they check only AUTO lines).
#[test]
fn a_manual_side_carrying_an_old_reference_is_stale() {
    let s = snapshot("MANUAL", "MANUAL", Some(30), Some(24));
    assert_eq!(
        Freshness::of_side(&s, Side::Buy, now()),
        stale(Some("2026-10-09"))
    );
    assert_eq!(
        Freshness::of_side(&s, Side::Sell, now()),
        stale(Some("2026-10-09"))
    );
}

#[test]
fn a_manual_side_with_no_reference_is_never_stale() {
    let s = snapshot("MANUAL", "MANUAL", None, Some(24));
    assert_eq!(Freshness::of_side(&s, Side::Buy, now()), fresh(None));
}

#[test]
fn no_age_limit_is_never_stale() {
    let s = snapshot("AUTO", "AUTO", Some(10_000), None);
    assert_eq!(
        Freshness::of_side(&s, Side::Sell, now()),
        fresh(Some("2025-08-19"))
    );
}

#[test]
fn an_unreadable_fetched_at_is_not_stale_and_keeps_its_date() {
    let mut s = snapshot("AUTO", "AUTO", Some(30), Some(24));
    s["fetched_at"] = json!("not a time");
    assert_eq!(
        Freshness::of_side(&s, Side::Buy, now()),
        fresh(Some("2026-10-09"))
    );
}

// ── a whole publication ───────────────────────────────────────────────────

#[test]
fn a_publication_is_stale_when_any_side_is() {
    let p = Freshness::of_publication([
        fresh(Some("2026-10-10")),
        stale(Some("2026-10-09")),
        fresh(None),
    ]);
    assert!(p.stale);
}

#[test]
fn a_publication_of_fresh_sides_is_fresh() {
    let p = Freshness::of_publication([fresh(Some("2026-10-10")), fresh(None)]);
    assert!(!p.stale);
}

#[test]
fn a_publications_source_date_is_the_oldest_not_the_first() {
    let p = Freshness::of_publication([
        fresh(Some("2026-10-10")),
        fresh(Some("2026-10-07")),
        fresh(None),
        fresh(Some("2026-10-09")),
    ]);
    assert_eq!(p.source_date, Some(date("2026-10-07")));
}

#[test]
fn a_publication_with_no_reference_has_no_source_date() {
    assert_eq!(
        Freshness::of_publication([fresh(None), fresh(None)]),
        fresh(None)
    );
    assert_eq!(
        Freshness::of_publication(std::iter::empty::<Freshness>()),
        fresh(None)
    );
}
