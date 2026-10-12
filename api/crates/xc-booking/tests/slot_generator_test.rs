//! BK-2 #1379 — `SlotGenerator::slots_for` (design `docs/design/blueprint/booking-slots.md` B2, §4.1, §9.1 row
//! BK-2: *"slots of `slot_minutes` from open to close in the branch timezone, none on a closure date, none on a
//! `Closed`/`NotSet` day, none past `horizon_days`"*; ticket §9 edge cases). Pure: no database.
//! Written before the implementation by the test writer (claude/dell-claude-4, on the BE lane controller
//! claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Pinned readings ([proposed] in the comment on #1379; the lane controller may overrule):
//! - `horizon_days` counts the first day: horizon 1 = `from` only (the table's CHECK is `BETWEEN 1 AND 90`, so a
//!   horizon of 0 cannot exist, which only makes sense if 1 already means "today");
//! - a slot that would end after the window's close is not offered (B2: slots are made *inside*
//!   `BranchHours::window`).

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc, Weekday};
use xc_booking::domain::slot::branch_closure::BranchClosure;
use xc_booking::domain::slot::slot_generator::{PlannedSlot, SlotGenerator};
use xc_booking::domain::slot::slot_rule::SlotRule;
use xc_rate::domain::branch::branch_hours::BranchHours;

const BANGKOK: &str = "Asia/Bangkok";

fn monday() -> NaiveDate {
    let d = NaiveDate::from_ymd_opt(2030, 1, 7).expect("a date");
    assert_eq!(d.weekday(), Weekday::Mon);
    d
}

fn rule(slot_minutes: i32, horizon_days: i32) -> SlotRule {
    SlotRule {
        slot_minutes,
        capacity: 3,
        horizon_days,
    }
}

/// The same window every day of the week.
fn every_day(open: &str, close: &str) -> serde_json::Value {
    let day = serde_json::json!([{ "open": open, "close": close }]);
    serde_json::json!({
        "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": day
    })
}

fn hours(tz: &str, value: Option<&serde_json::Value>) -> BranchHours {
    BranchHours::new(tz, value).expect("valid hours")
}

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
        .single()
        .expect("an instant")
}

fn dates(slots: &[PlannedSlot]) -> Vec<NaiveDate> {
    let mut d: Vec<NaiveDate> = slots.iter().map(|s| s.service_date).collect();
    d.dedup();
    d
}

#[test]
fn slots_of_slot_minutes_run_from_open_to_close_in_the_branch_timezone() {
    // Bangkok is UTC+7: 09:00–12:00 local is 02:00–05:00Z.
    let h = hours(BANGKOK, Some(&every_day("09:00", "12:00")));
    let slots = SlotGenerator::slots_for(&rule(30, 1), &h, &[], monday(), 1);
    let starts: Vec<DateTime<Utc>> = slots.iter().map(|s| s.start_at).collect();
    assert_eq!(
        starts,
        vec![
            utc(2030, 1, 7, 2, 0),
            utc(2030, 1, 7, 2, 30),
            utc(2030, 1, 7, 3, 0),
            utc(2030, 1, 7, 3, 30),
            utc(2030, 1, 7, 4, 0),
            utc(2030, 1, 7, 4, 30),
        ],
        "six 30-minute slots from 09:00 to 12:00 Bangkok"
    );
    for s in &slots {
        assert_eq!(s.end_at - s.start_at, Duration::minutes(30), "{s:?}");
        assert_eq!(s.service_date, monday(), "{s:?}");
    }
}

#[test]
fn a_slot_that_would_end_after_close_is_not_offered() {
    // 09:00–10:45 with 30-minute slots: 09:00, 09:30, 10:00; a 10:30 slot would end at 11:00.
    let h = hours(BANGKOK, Some(&every_day("09:00", "10:45")));
    let slots = SlotGenerator::slots_for(&rule(30, 1), &h, &[], monday(), 1);
    assert_eq!(slots.len(), 3, "{slots:?}");
    assert_eq!(slots.last().expect("a slot").end_at, utc(2030, 1, 7, 3, 30));
    let close = utc(2030, 1, 7, 3, 45);
    assert!(slots.iter().all(|s| s.end_at <= close), "{slots:?}");
}

#[test]
fn the_service_date_is_the_branch_local_date_east_of_utc() {
    // 06:00 Bangkok on Monday is 23:00Z on Sunday: the slot belongs to Monday.
    let h = hours(BANGKOK, Some(&every_day("06:00", "07:00")));
    let slots = SlotGenerator::slots_for(&rule(60, 1), &h, &[], monday(), 1);
    assert_eq!(slots.len(), 1, "{slots:?}");
    assert_eq!(slots[0].start_at, utc(2030, 1, 6, 23, 0));
    assert_eq!(
        slots[0].service_date,
        monday(),
        "the branch's date, not UTC's"
    );
}

#[test]
fn the_service_date_is_the_branch_local_date_west_of_utc() {
    // 18:00 Los Angeles (UTC−8 in January) on Monday is 02:00Z on Tuesday: the slot belongs to Monday.
    let h = hours("America/Los_Angeles", Some(&every_day("18:00", "19:00")));
    let slots = SlotGenerator::slots_for(&rule(60, 1), &h, &[], monday(), 1);
    assert_eq!(slots.len(), 1, "{slots:?}");
    assert_eq!(slots[0].start_at, utc(2030, 1, 8, 2, 0));
    assert_eq!(
        slots[0].service_date,
        monday(),
        "the branch's date, not UTC's"
    );
}

#[test]
fn a_closure_date_has_no_slots() {
    let h = hours(BANGKOK, Some(&every_day("09:00", "11:00")));
    let tuesday = monday().succ_opt().expect("a date");
    let closures = [BranchClosure {
        closed_on: tuesday,
        reason: "Holiday".to_owned(),
    }];
    let slots = SlotGenerator::slots_for(&rule(60, 3), &h, &closures, monday(), 3);
    let wednesday = tuesday.succ_opt().expect("a date");
    assert_eq!(dates(&slots), vec![monday(), wednesday], "{slots:?}");
    assert_eq!(slots.len(), 4, "two a day on the open days");
}

#[test]
fn a_closed_day_has_no_slots() {
    // Hours on Monday and Wednesday only: Tuesday is `Closed`.
    let day = serde_json::json!([{ "open": "09:00", "close": "11:00" }]);
    let h = hours(
        BANGKOK,
        Some(&serde_json::json!({ "mon": day, "wed": day })),
    );
    let tuesday = monday().succ_opt().expect("a date");
    assert!(matches!(
        h.window(tuesday),
        xc_rate::domain::branch::branch_hours::HoursWindow::Closed
    ));
    let slots = SlotGenerator::slots_for(&rule(60, 3), &h, &[], monday(), 3);
    assert_eq!(
        dates(&slots),
        vec![monday(), tuesday.succ_opt().expect("a date")],
        "{slots:?}"
    );
}

#[test]
fn a_branch_whose_hours_are_not_set_has_no_slots() {
    let h = hours(BANGKOK, None);
    let slots = SlotGenerator::slots_for(&rule(30, 90), &h, &[], monday(), 90);
    assert!(slots.is_empty(), "{slots:?}");
}

#[test]
fn no_slot_lies_past_horizon_days_counting_the_first_day() {
    let h = hours(BANGKOK, Some(&every_day("09:00", "10:00")));
    let slots = SlotGenerator::slots_for(&rule(60, 3), &h, &[], monday(), 10);
    let expected: Vec<NaiveDate> = monday().iter_days().take(3).collect();
    assert_eq!(dates(&slots), expected, "a horizon of 3 days opens 3 dates");
    let one = SlotGenerator::slots_for(&rule(60, 1), &h, &[], monday(), 10);
    assert_eq!(
        dates(&one),
        vec![monday()],
        "a horizon of 1 opens the first date only"
    );
}

#[test]
fn fewer_days_than_the_horizon_open_only_those_days() {
    let h = hours(BANGKOK, Some(&every_day("09:00", "10:00")));
    let slots = SlotGenerator::slots_for(&rule(60, 14), &h, &[], monday(), 2);
    let expected: Vec<NaiveDate> = monday().iter_days().take(2).collect();
    assert_eq!(dates(&slots), expected);
}

#[test]
fn the_slots_come_back_in_start_order() {
    let h = hours(BANGKOK, Some(&every_day("09:00", "12:00")));
    let slots = SlotGenerator::slots_for(&rule(45, 5), &h, &[], monday(), 5);
    assert_eq!(
        slots.len(),
        5 * 4,
        "four 45-minute slots fit in 3 hours, for 5 days"
    );
    assert!(
        slots.windows(2).all(|w| w[0].start_at < w[1].start_at),
        "{slots:?}"
    );
}
