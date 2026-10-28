//! XC-V9-PRICING PV-g (pricing-tiers.md §15.5; `H:` §6): `SourceFreshness::of_feed`, the one place v3's market feed
//! is read as "the last fetch succeeded". Found by gemini/studio-5's block on #5166: `MarketRefresh::tick` records the
//! attempt at its own `now` and writes a new batch only when the answer changed, so comparing the newest batch's
//! `fetched_at` with `last_attempt_at` blocked every FOLLOW currency as STALE_SOURCE. Written before the
//! implementation by the sub-controller claude/451f0116. Do not edit, delete or weaken; add tests in a file of your own.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_rate::domain::pricing::follow_plan::SourceFreshness;

fn t(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 5, minute, 0).unwrap()
}

/// PV-g: a successful attempt after the newest batch (an unchanged answer, or a batch stamped a moment earlier inside
/// `fetch`) makes the source current as of that attempt.
#[test]
fn a_successful_attempt_after_the_newest_batch_is_current_as_of_the_attempt() {
    let f = SourceFreshness::of_feed(Some(t(0)), Some(t(40)), None);
    assert_eq!(f.fetched_at, Some(t(40)));
    assert_eq!(f.last_attempt_at, Some(t(40)));
    assert!(f.is_current(t(45)));
    assert!(!f.is_current(t(40) + Duration::minutes(31)));
}

/// PV-g: a failed last attempt keeps the newest batch's time and the error, so the source is not current.
#[test]
fn a_failed_last_attempt_is_not_current() {
    let f = SourceFreshness::of_feed(Some(t(0)), Some(t(5)), Some("TT: HTTP 503".into()));
    assert_eq!(f.fetched_at, Some(t(0)));
    assert_eq!(f.error.as_deref(), Some("TT: HTTP 503"));
    assert!(!f.is_current(t(6)));
}

/// PV-g: no batch at all is never current, whatever the attempt says (no snapshot identity, `H:` §6).
#[test]
fn no_batch_is_never_current() {
    let f = SourceFreshness::of_feed(None, Some(t(5)), None);
    assert_eq!(f.fetched_at, None);
    assert!(!f.is_current(t(6)));
}

/// PV-g: a source never attempted keeps the batch time and no attempt, so it is not current.
#[test]
fn a_source_never_attempted_is_not_current() {
    let f = SourceFreshness::of_feed(Some(t(0)), None, None);
    assert_eq!(f.last_attempt_at, None);
    assert!(!f.is_current(t(1)));
}
