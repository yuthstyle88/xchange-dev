//! Up-front tests for CS-20 (design `docs/design/configurable-shifts.md` §2.10, T9): `ShiftPlan::business_day_end(d)`
//! is the end of `service_span(d)`. Written by the ticket author claude/35271802 — the implementer must not edit,
//! delete or weaken these.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::America::New_York;
use chrono_tz::Asia::Bangkok;
use xc_shift::domain::shift_plan::shift_plan::ShiftPlan;
use xc_shift::domain::shift_plan::shift_template::ShiftTemplate;

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn utc(iso: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(iso)
        .unwrap()
        .with_timezone(&Utc)
}

fn template(name: &str, start: NaiveTime, end: NaiveTime, position: i16) -> ShiftTemplate {
    ShiftTemplate {
        id: name.to_owned(),
        name: name.to_owned(),
        starts_at: start,
        ends_at: end,
        break_window: None,
        position,
    }
}

/// The owner's defaults: A 06–14, B 14–22, C 22–06 in Bangkok.
fn defaults() -> ShiftPlan {
    ShiftPlan::new(
        vec![
            template("A", hm(6, 0), hm(14, 0), 0),
            template("B", hm(14, 0), hm(22, 0), 1),
            template("C", hm(22, 0), hm(6, 0), 2),
        ],
        Bangkok,
    )
}

/// E1: the 4 Nov business day ends at 5 Nov 06:00 Bangkok, 4 Nov 23:00 UTC; not at midnight.
#[test]
fn e1_the_defaults_day_ends_at_six_the_next_morning() {
    assert_eq!(
        defaults().business_day_end(day(2026, 11, 4)),
        Some(utc("2026-11-04T23:00:00Z"))
    );
}

/// E2: it is the end of the day's service span, so a 5 Nov 02:00 instant belongs to a day that has not ended yet.
#[test]
fn e2_it_is_the_end_of_the_service_span() {
    let p = defaults();
    let d = day(2026, 11, 4);
    let (_, end) = p.service_span(d).expect("a span");
    assert_eq!(p.business_day_end(d), Some(end));
    let two_am = utc("2026-11-04T19:00:00Z");
    assert_eq!(p.business_date_of(two_am), Some(d));
    assert!(two_am < p.business_day_end(d).unwrap());
}

/// E3: an empty plan has no business day, so no end.
#[test]
fn e3_an_empty_plan_has_no_end() {
    assert_eq!(
        ShiftPlan::new(Vec::new(), Bangkok).business_day_end(day(2026, 11, 4)),
        None
    );
}

/// E4: a DST zone (the CS-1c plan, cutover 02:30 New York): the 7 March day ends where the 8 March day starts, at the
/// end of the spring-forward gap, 03:00 EDT = 07:00 UTC.
#[test]
fn e4_across_a_dst_gap_the_day_ends_at_the_end_of_the_gap() {
    let p = ShiftPlan::new(
        vec![
            template("N", hm(2, 30), hm(14, 30), 0),
            template("D", hm(14, 30), hm(2, 30), 1),
        ],
        New_York,
    );
    assert_eq!(
        p.business_day_end(day(2026, 3, 7)),
        Some(utc("2026-03-08T07:00:00Z"))
    );
}
