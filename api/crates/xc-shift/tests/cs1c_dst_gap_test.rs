//! Up-front tests for CS-1c (review note on PR 6171 L63, via the lane controller claude/022a7453): a shift boundary
//! whose local time falls in a daylight-saving gap starts at the end of the gap, in the branch's timezone. Written by
//! the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//!
//! New York springs forward on 2026-03-08: 02:00 EST jumps to 03:00 EDT, so 02:30 does not exist that day. The plan is
//! N 02:30–14:30, D 14:30–02:30 (cutover 02:30, all 24 hours). Bangkok has no DST, so the owner's branches never hit
//! this; the rule only keeps a custom plan in a DST zone from moving by hours.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::America::New_York;
use xc_shift::domain::shift_plan::shift_plan::ShiftPlan;
use xc_shift::domain::shift_plan::shift_template::ShiftTemplate;

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn march(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
}

fn utc(iso: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(iso).unwrap().with_timezone(&Utc)
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

fn plan() -> ShiftPlan {
    ShiftPlan::new(
        vec![
            template("N", hm(2, 30), hm(14, 30), 0),
            template("D", hm(14, 30), hm(2, 30), 1),
        ],
        New_York,
    )
}

fn n(plan: &ShiftPlan) -> ShiftTemplate {
    plan.templates()[0].clone()
}

fn d(plan: &ShiftPlan) -> ShiftTemplate {
    plan.templates()[1].clone()
}

/// G1: on the spring-forward day N's 02:30 does not exist; N starts at the end of the gap, 03:00 EDT (07:00 UTC).
#[test]
fn g1_a_start_in_the_gap_is_the_end_of_the_gap() {
    let p = plan();
    assert_eq!(
        p.instance(&n(&p), march(8)).starts_at,
        utc("2026-03-08T07:00:00Z")
    );
}

/// G2: the day before has no gap; N starts at 02:30 EST (07:30 UTC), as today.
#[test]
fn g2_a_normal_day_is_unchanged() {
    let p = plan();
    assert_eq!(
        p.instance(&n(&p), march(7)).starts_at,
        utc("2026-03-07T07:30:00Z")
    );
}

/// G3: D of 7 March ends at the same gap, so it ends exactly when N of 8 March starts: no hole, no overlap.
#[test]
fn g3_the_shift_ending_in_the_gap_ends_where_the_next_one_starts() {
    let p = plan();
    let d7 = p.instance(&d(&p), march(7));
    let n8 = p.instance(&n(&p), march(8));
    assert_eq!(d7.ends_at, utc("2026-03-08T07:00:00Z"));
    assert_eq!(d7.ends_at, n8.starts_at);
}

/// G4: an instant inside the gap's hour belongs to the business day it falls in: 06:45 UTC on 8 March is 01:45 EST,
/// still the 7 March business day (D), and 07:00 UTC is the first instant of the 8 March one.
#[test]
fn g4_the_business_date_around_the_gap() {
    let p = plan();
    assert_eq!(
        p.business_date_of(utc("2026-03-08T06:45:00Z")),
        Some(march(7))
    );
    assert_eq!(
        p.business_date_of(utc("2026-03-08T07:00:00Z")),
        Some(march(8))
    );
}
