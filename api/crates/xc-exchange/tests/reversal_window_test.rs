//! Ticket #682 (RV-1+RV-2), Part 1 (RV-1) — the reversal window (R-1): a reversal may be requested
//! and posted only within the branch business day of the original's completion, in the branch's
//! timezone.
//! Written before the implementation by the ticket author for the TX/RV chain (claude/f03345fe, on
//! the lane controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/reversal.md` §0 R-1 (*"ภายในวันทำการเดียวกัน"*), §4.1 (both request
//! and post refused `reversalWindowClosed` once "now" in the branch timezone is past the original's
//! `business_date`).
//!
//! Signatures this test assumes, [required] (finding RG2 on #682, per lane controller 2026-10-10:
//! `BranchHours` lives in xc-rate, which xc-exchange cannot depend on; the window takes the branch's
//! IANA timezone):
//! - `xc_exchange::domain::reversal::window::ReversalWindow::business_date_of(completed_at:
//!   DateTime<Utc>, tz: chrono_tz::Tz) -> NaiveDate`: the date of `completed_at` in `tz`;
//! - `ReversalWindow::is_open(business_date: NaiveDate, now: DateTime<Utc>, tz: chrono_tz::Tz) ->
//!   bool`: true while the date of `now` in `tz` is not after `business_date`.

use chrono::{NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use xc_exchange::domain::reversal::window::ReversalWindow;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

const BANGKOK: Tz = chrono_tz::Asia::Bangkok;
const LONDON: Tz = chrono_tz::Europe::London;

#[test]
fn the_business_date_is_the_completion_date_in_the_branch_timezone() {
    // 2026-10-06 16:59:59Z is 23:59:59 in Bangkok; one second later it is the 7th there
    let before = Utc.with_ymd_and_hms(2026, 10, 6, 16, 59, 59).unwrap();
    let after = Utc.with_ymd_and_hms(2026, 10, 6, 17, 0, 0).unwrap();
    assert_eq!(
        ReversalWindow::business_date_of(before, BANGKOK),
        date(2026, 10, 6)
    );
    assert_eq!(
        ReversalWindow::business_date_of(after, BANGKOK),
        date(2026, 10, 7)
    );
    // the same instant is still the 6th in London (BST, UTC+1)
    assert_eq!(
        ReversalWindow::business_date_of(after, LONDON),
        date(2026, 10, 6)
    );
}

#[test]
fn the_window_is_open_until_the_end_of_the_business_date_in_the_branch_timezone() {
    let day = date(2026, 10, 6);
    let completed = Utc.with_ymd_and_hms(2026, 10, 6, 3, 0, 0).unwrap();
    let last_second = Utc.with_ymd_and_hms(2026, 10, 6, 16, 59, 59).unwrap();
    let midnight = Utc.with_ymd_and_hms(2026, 10, 6, 17, 0, 0).unwrap();
    let next_week = Utc.with_ymd_and_hms(2026, 10, 13, 3, 0, 0).unwrap();
    assert!(
        ReversalWindow::is_open(day, completed, BANGKOK),
        "right after the completion"
    );
    assert!(
        ReversalWindow::is_open(day, last_second, BANGKOK),
        "23:59:59 in Bangkok"
    );
    assert!(
        !ReversalWindow::is_open(day, midnight, BANGKOK),
        "00:00 the next day in Bangkok: closed"
    );
    assert!(
        !ReversalWindow::is_open(day, next_week, BANGKOK),
        "a week later"
    );
}

#[test]
fn the_timezone_decides_when_the_day_ends() {
    let day = date(2026, 10, 6);
    // 17:30Z: the 7th in Bangkok, still the 6th in London
    let now = Utc.with_ymd_and_hms(2026, 10, 6, 17, 30, 0).unwrap();
    assert!(!ReversalWindow::is_open(day, now, BANGKOK), "Bangkok");
    assert!(ReversalWindow::is_open(day, now, LONDON), "London");
}
