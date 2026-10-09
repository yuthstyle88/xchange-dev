//! Ticket #615 (OR-1) — a branch's opening hours for one date, in the branch's own timezone (`BranchHours::window`).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) §6 (the `opening_hours` shape
//! [required]: `{"mon":[{"open":"11:00","close":"23:00"}], …, "sun":[]}`, an empty day is closed, `NULL` is "not
//! set"; `window(date) -> HoursWindow { Closed | NotSet | Open { open, close } }` uses the branch timezone), §7
//! (`BranchHours` in `xc-rate/src/domain/branch/branch_hours.rs`), D-6 and the owner's B4/B5 (§0): a branch with
//! no hours is "not set", never a default; §11: business date by branch timezone. §10 OR-1: across the branch
//! timezone's midnight; `NotSet` for NULL hours; an empty day is `Closed`.
//!
//! Signatures these tests assume ([proposed] where the design does not state them):
//! - `xc_rate::domain::branch::branch_hours::{BranchHours, HoursWindow}` (the §7 file).
//! - `BranchHours::new(timezone: &str, opening_hours: Option<&serde_json::Value>) -> pos_error::AppResult<BranchHours>`
//!   [proposed]. `timezone` is the IANA name stored in `branches.timezone`; `opening_hours` is the
//!   `branches.opening_hours` jsonb, `None` being SQL NULL. An unknown timezone, or a value that is not the §6
//!   shape, is an `Err`.
//! - `BranchHours::window(&self, date: chrono::NaiveDate) -> HoursWindow` (§6). `date` is a calendar date in the
//!   branch timezone, and its weekday picks the `opening_hours` key (`mon` … `sun`).
//! - `enum HoursWindow { Closed, NotSet, Open { open: DateTime<Utc>, close: DateTime<Utc> } }`, deriving `Debug`,
//!   `PartialEq` and `Eq` [proposed: `open` and `close` are instants, so the timezone is applied inside `window`,
//!   and a caller compares them with `Utc::now()`].
//! - A window belongs to the date it opens on. A `close` earlier than its `open` (`"18:00"`–`"02:00"`, or
//!   `"11:00"`–`"00:00"`) is on the next day [proposed: the design's "a window that crosses the branch timezone's
//!   midnight"].
//!
//! Not pinned here, because the design does not say: a day listing more than one window, a day key missing from
//! the object, and a local time that a daylight-saving change skips.

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};
use xc_rate::domain::branch::branch_hours::{BranchHours, HoursWindow};

const BANGKOK: &str = "Asia/Bangkok";

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"))
        .with_timezone(&Utc)
}

fn hours(timezone: &str, opening_hours: &Value) -> BranchHours {
    BranchHours::new(timezone, Some(opening_hours))
        .unwrap_or_else(|e| panic!("{timezone} {opening_hours}: {e:?}"))
}

fn open(from: &str, to: &str) -> HoursWindow {
    HoursWindow::Open {
        open: utc(from),
        close: utc(to),
    }
}

fn contains(window: HoursWindow, t: DateTime<Utc>) -> bool {
    match window {
        HoursWindow::Open { open, close } => open <= t && t < close,
        HoursWindow::Closed | HoursWindow::NotSet => false,
    }
}

/// The same hours every day of the week, as §6 writes them.
fn every_day(open: &str, close: &str) -> Value {
    let one = json!([{ "open": open, "close": close }]);
    json!({
        "mon": one, "tue": one, "wed": one, "thu": one, "fri": one, "sat": one, "sun": one,
    })
}

// 2026-10-12 is a Monday and 2026-10-18 a Sunday. Bangkok is UTC+7 all year.

#[test]
fn the_section_6_shape_gives_each_weekday_its_own_window() {
    let week = json!({
        "mon": [{ "open": "11:00", "close": "23:00" }],
        "tue": [{ "open": "10:00", "close": "22:00" }],
        "wed": [{ "open": "09:30", "close": "21:30" }],
        "thu": [{ "open": "12:00", "close": "20:00" }],
        "fri": [{ "open": "08:00", "close": "18:00" }],
        "sat": [{ "open": "10:00", "close": "16:00" }],
        "sun": [],
    });
    let h = hours(BANGKOK, &week);
    for (date, want) in [
        (
            "2026-10-12",
            open("2026-10-12T04:00:00Z", "2026-10-12T16:00:00Z"),
        ),
        (
            "2026-10-13",
            open("2026-10-13T03:00:00Z", "2026-10-13T15:00:00Z"),
        ),
        (
            "2026-10-14",
            open("2026-10-14T02:30:00Z", "2026-10-14T14:30:00Z"),
        ),
        (
            "2026-10-15",
            open("2026-10-15T05:00:00Z", "2026-10-15T13:00:00Z"),
        ),
        (
            "2026-10-16",
            open("2026-10-16T01:00:00Z", "2026-10-16T11:00:00Z"),
        ),
        (
            "2026-10-17",
            open("2026-10-17T03:00:00Z", "2026-10-17T09:00:00Z"),
        ),
        ("2026-10-18", HoursWindow::Closed),
        // The next week reads the same keys.
        (
            "2026-10-19",
            open("2026-10-19T04:00:00Z", "2026-10-19T16:00:00Z"),
        ),
        ("2026-10-25", HoursWindow::Closed),
    ] {
        assert_eq!(h.window(day(date)), want, "{date}");
    }
}

#[test]
fn null_hours_are_not_set_on_every_day_and_never_a_default() {
    for timezone in [BANGKOK, "Asia/Tokyo", "Europe/London"] {
        let h = BranchHours::new(timezone, None).unwrap_or_else(|e| panic!("{timezone}: {e:?}"));
        for date in [
            "2026-10-12",
            "2026-10-13",
            "2026-10-14",
            "2026-10-15",
            "2026-10-16",
            "2026-10-17",
            "2026-10-18",
        ] {
            assert_eq!(
                h.window(day(date)),
                HoursWindow::NotSet,
                "{timezone} {date}"
            );
        }
    }
}

#[test]
fn an_empty_day_is_closed_and_not_not_set() {
    let mut week = every_day("11:00", "23:00");
    week["sun"] = json!([]);
    let h = hours(BANGKOK, &week);
    assert_eq!(h.window(day("2026-10-18")), HoursWindow::Closed);
    assert_eq!(
        h.window(day("2026-10-17")),
        open("2026-10-17T04:00:00Z", "2026-10-17T16:00:00Z"),
        "the day before is open: Closed is the empty day's answer only"
    );
    assert_eq!(
        h.window(day("2026-10-19")),
        open("2026-10-19T04:00:00Z", "2026-10-19T16:00:00Z"),
        "the day after is open"
    );

    let closed_all_week = json!({
        "mon": [], "tue": [], "wed": [], "thu": [], "fri": [], "sat": [], "sun": [],
    });
    let h = hours(BANGKOK, &closed_all_week);
    assert_eq!(
        h.window(day("2026-10-14")),
        HoursWindow::Closed,
        "hours that are set but empty are Closed, not NotSet"
    );
}

#[test]
fn a_window_that_crosses_the_branch_midnight_closes_on_the_next_day() {
    let mut week = every_day("11:00", "23:00");
    week["fri"] = json!([{ "open": "18:00", "close": "02:00" }]);
    week["sat"] = json!([]);
    week["mon"] = json!([{ "open": "11:00", "close": "00:00" }]);
    let h = hours(BANGKOK, &week);

    // Friday 18:00 to Saturday 02:00 in Bangkok.
    assert_eq!(
        h.window(day("2026-10-16")),
        open("2026-10-16T11:00:00Z", "2026-10-16T19:00:00Z"),
    );
    // The window belongs to the Friday it opens on: Saturday's own entry is empty.
    assert_eq!(h.window(day("2026-10-17")), HoursWindow::Closed);
    // Saturday 01:30 in Bangkok is still inside Friday's window.
    assert!(contains(
        h.window(day("2026-10-16")),
        utc("2026-10-16T18:30:00Z")
    ));
    assert!(!contains(
        h.window(day("2026-10-16")),
        utc("2026-10-16T19:00:00Z")
    ));

    // Closing at midnight is the end of that day: Tuesday 00:00 in Bangkok.
    assert_eq!(
        h.window(day("2026-10-12")),
        open("2026-10-12T04:00:00Z", "2026-10-12T17:00:00Z"),
    );
}

#[test]
fn a_window_that_crosses_the_utc_midnight_but_not_the_branch_midnight_stays_on_its_date() {
    // Tokyo is UTC+9: 08:00–20:00 local is 23:00Z the day before to 11:00Z.
    let h = hours("Asia/Tokyo", &every_day("08:00", "20:00"));
    assert_eq!(
        h.window(day("2026-10-14")),
        open("2026-10-13T23:00:00Z", "2026-10-14T11:00:00Z"),
    );
}

#[test]
fn the_timezone_changes_the_window_of_the_same_date() {
    let week = every_day("11:00", "23:00");
    for (timezone, want) in [
        (
            BANGKOK,
            open("2026-10-12T04:00:00Z", "2026-10-12T16:00:00Z"),
        ),
        (
            "Asia/Tokyo",
            open("2026-10-12T02:00:00Z", "2026-10-12T14:00:00Z"),
        ),
        // London is on summer time (UTC+1) until 2026-10-25.
        (
            "Europe/London",
            open("2026-10-12T10:00:00Z", "2026-10-12T22:00:00Z"),
        ),
    ] {
        assert_eq!(
            hours(timezone, &week).window(day("2026-10-12")),
            want,
            "{timezone}"
        );
    }
}

#[test]
fn the_same_instant_is_inside_the_hours_in_one_timezone_and_outside_in_another() {
    let week = every_day("11:00", "23:00");
    // Monday 22:30 in Bangkok; Tuesday 00:30 in Tokyo.
    let t = utc("2026-10-12T15:30:00Z");
    let monday = day("2026-10-12");
    let tuesday = day("2026-10-13");

    let bangkok = hours(BANGKOK, &week);
    assert!(
        contains(bangkok.window(monday), t),
        "Bangkok: inside Monday's 11:00–23:00"
    );

    let tokyo = hours("Asia/Tokyo", &week);
    assert!(
        !contains(tokyo.window(monday), t),
        "Tokyo: Monday's window closed at 23:00 local (14:00Z)"
    );
    assert!(
        !contains(tokyo.window(tuesday), t),
        "Tokyo: Tuesday's window opens at 11:00 local (02:00Z)"
    );
}

#[test]
fn the_timezone_is_the_iana_zone_with_its_daylight_saving_rules() {
    // Europe/London moves from UTC+0 to UTC+1 on Sunday 2026-03-29 at 01:00Z.
    // A fixed offset cannot give both answers.
    let h = hours("Europe/London", &every_day("11:00", "23:00"));
    assert_eq!(
        h.window(day("2026-03-28")),
        open("2026-03-28T11:00:00Z", "2026-03-28T23:00:00Z"),
    );
    assert_eq!(
        h.window(day("2026-03-29")),
        open("2026-03-29T10:00:00Z", "2026-03-29T22:00:00Z"),
    );
}

#[test]
fn an_unknown_timezone_is_refused() {
    let week = every_day("11:00", "23:00");
    for timezone in ["", "Mars/Olympus_Mons", "Bangkok", "asia/bangkok_x"] {
        assert!(
            BranchHours::new(timezone, Some(&week)).is_err(),
            "{timezone:?} is not an IANA zone"
        );
        assert!(
            BranchHours::new(timezone, None).is_err(),
            "{timezone:?} is refused even with no hours set"
        );
    }
}

#[test]
fn hours_outside_the_section_6_shape_are_refused() {
    for (what, value) in [
        ("an array instead of the weekly object", json!([])),
        (
            "a string instead of the weekly object",
            json!("11:00-23:00"),
        ),
        (
            "a day that is a string, not a list",
            json!({ "mon": "11:00-23:00" }),
        ),
        (
            "an entry without close",
            json!({ "mon": [{ "open": "11:00" }] }),
        ),
        (
            "an entry without open",
            json!({ "mon": [{ "close": "23:00" }] }),
        ),
        (
            "an hour past 23",
            json!({ "mon": [{ "open": "25:00", "close": "23:00" }] }),
        ),
        (
            "a minute past 59",
            json!({ "mon": [{ "open": "11:00", "close": "23:60" }] }),
        ),
        (
            "a time that is a number",
            json!({ "mon": [{ "open": 11, "close": 23 }] }),
        ),
        (
            "a time that is not HH:MM",
            json!({ "mon": [{ "open": "11am", "close": "11pm" }] }),
        ),
    ] {
        assert!(
            BranchHours::new(BANGKOK, Some(&value)).is_err(),
            "{what}: {value}"
        );
    }
}
