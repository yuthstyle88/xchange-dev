//! Up-front tests for `ShiftPlan` (docs/design/configurable-shifts.md §2.2, §4 CS-1 / CS-1b, rows P1–P20).
//! Written by the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//!
//! Owner 2026-11-04: *"มาตรฐานการทำงาน 24 ชม ไม่มีพัก"* and *"ให้มัน คร 24"*: a plan covers all 24 hours. The cutover
//! is the earliest shift start; a business day is the 24 hours from it; a shift that straddles the cutover belongs to
//! the day it started. Most rows use an overlapping 24-hour fixture: A 06–15, B 14–23, C 22–07 (cutover 06:00).

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Asia::Bangkok;
use chrono_tz::Tz;
use xc_shift::domain::shift_plan::plan_refusal::PlanRefusal;
use xc_shift::domain::shift_plan::shift_plan::ShiftPlan;
use xc_shift::domain::shift_plan::shift_template::{ShiftTemplate, ShiftTemplateInput};

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn date(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

/// Bangkok wall-clock instant on 2026-11-`d`.
fn bkk(d: u32, h: u32, m: u32) -> DateTime<Utc> {
    Bangkok
        .with_ymd_and_hms(2026, 11, d, h, m, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn input(name: &str, start: NaiveTime, end: NaiveTime) -> ShiftTemplateInput {
    ShiftTemplateInput {
        id: None,
        name: name.to_owned(),
        starts_at: start,
        ends_at: end,
        break_window: None,
    }
}

fn with_break(mut t: ShiftTemplateInput, start: NaiveTime, end: NaiveTime) -> ShiftTemplateInput {
    t.break_window = Some((start, end));
    t
}

fn templates_from(inputs: Vec<ShiftTemplateInput>) -> Vec<ShiftTemplate> {
    inputs
        .into_iter()
        .enumerate()
        .map(|(i, t)| ShiftTemplate {
            id: t.name.clone(),
            name: t.name,
            starts_at: t.starts_at,
            ends_at: t.ends_at,
            break_window: t.break_window,
            position: i as i16,
        })
        .collect()
}

/// A 06–15, B 14–23 (break 18–19), C 22–07: overlapping, all 24 hours covered.
fn overlap24_inputs() -> Vec<ShiftTemplateInput> {
    vec![
        input("A", hm(6, 0), hm(15, 0)),
        with_break(input("B", hm(14, 0), hm(23, 0)), hm(18, 0), hm(19, 0)),
        input("C", hm(22, 0), hm(7, 0)),
    ]
}

fn overlap24_in(tz: Tz) -> ShiftPlan {
    ShiftPlan::new(templates_from(overlap24_inputs()), tz)
}

fn overlap24() -> ShiftPlan {
    overlap24_in(Bangkok)
}

fn owner_defaults() -> ShiftPlan {
    ShiftPlan::new(templates_from(ShiftPlan::defaults()), Bangkok)
}

/// `(template id, business date)` of each covering instance, in order.
fn covering(plan: &ShiftPlan, at: DateTime<Utc>) -> Vec<(String, NaiveDate)> {
    plan.covering(at)
        .into_iter()
        .map(|i| (i.template_id, i.business_date))
        .collect()
}

fn named(id: &str, d: u32) -> (String, NaiveDate) {
    (id.to_owned(), date(d))
}

#[test]
fn p1_just_after_the_cutover_the_new_morning_shift_and_last_nights_shift_both_cover() {
    assert_eq!(
        covering(&overlap24(), bkk(4, 6, 30)),
        vec![named("A", 4), named("C", 3)]
    );
}

#[test]
fn p2_afternoon_overlap_is_a_and_b() {
    assert_eq!(
        covering(&overlap24(), bkk(4, 14, 30)),
        vec![named("A", 4), named("B", 4)]
    );
}

#[test]
fn p3_evening_overlap_is_b_and_c() {
    assert_eq!(
        covering(&overlap24(), bkk(4, 22, 30)),
        vec![named("B", 4), named("C", 4)]
    );
}

#[test]
fn p4_two_am_is_shift_c_of_the_previous_business_date() {
    assert_eq!(covering(&overlap24(), bkk(5, 2, 0)), vec![named("C", 4)]);
}

#[test]
fn p5_a_shift_ends_exclusive() {
    assert_eq!(covering(&overlap24(), bkk(4, 15, 0)), vec![named("B", 4)]);
}

#[test]
fn p6_some_shift_covers_every_minute_of_a_valid_plan() {
    let plan = overlap24();
    for minute in (0..24 * 60).step_by(7) {
        let at = bkk(4, 0, 0) + chrono::Duration::minutes(minute);
        assert!(!plan.covering(at).is_empty(), "{at}");
    }
}

#[test]
fn p7_the_business_date_cuts_over_at_the_earliest_start() {
    let plan = overlap24();
    assert_eq!(plan.business_date_of(bkk(5, 2, 0)), Some(date(4)));
    assert_eq!(plan.business_date_of(bkk(5, 5, 59)), Some(date(4)));
    assert_eq!(plan.business_date_of(bkk(5, 6, 0)), Some(date(5)));
    // C of 4 Nov still covers 06:30 on 5 Nov, but the business date has moved on.
    assert_eq!(plan.business_date_of(bkk(5, 6, 30)), Some(date(5)));
}

#[test]
fn p8_a_business_day_is_the_24_hours_from_the_cutover() {
    let span = overlap24().service_span(date(4));
    assert_eq!(span, Some((bkk(4, 6, 0), bkk(5, 6, 0))));
    assert_eq!(
        span.map(|(a, b)| (a.to_rfc3339(), b.to_rfc3339())),
        Some((
            "2026-11-03T23:00:00+00:00".to_owned(),
            "2026-11-04T23:00:00+00:00".to_owned()
        ))
    );
}

#[test]
fn p9_span_for_is_the_span_of_the_business_date_of_now() {
    let plan = overlap24();
    assert_eq!(
        plan.span_for(bkk(5, 2, 0)),
        Some((bkk(4, 6, 0), bkk(5, 6, 0)))
    );
    assert_eq!(
        plan.span_for(bkk(5, 6, 0)),
        Some((bkk(5, 6, 0), bkk(6, 6, 0)))
    );
}

#[test]
fn p10_on_break_only_where_a_break_is_set() {
    let plan = overlap24();
    assert!(plan.on_break("B", bkk(4, 18, 30)));
    assert!(!plan.on_break("B", bkk(4, 19, 0)));
    assert!(!plan.on_break("A", bkk(4, 14, 30)), "A has no break");
    assert!(
        !owner_defaults().on_break("C", bkk(5, 2, 0)),
        "the defaults have no breaks"
    );
}

#[test]
fn p11_a_shift_straddling_the_cutover_belongs_to_the_day_it_started() {
    let plan = overlap24();
    let c = plan.templates()[2].clone();
    let i = plan.instance(&c, date(4));
    assert_eq!(i.template_id, "C");
    assert_eq!(i.business_date, date(4));
    assert_eq!(i.starts_at, bkk(4, 22, 0));
    assert_eq!(i.ends_at, bkk(5, 7, 0));
    assert_eq!(i.break_window, None);
}

#[test]
fn p12_validate_refuses_each_broken_list() {
    let day = || input("A", hm(6, 0), hm(18, 0));
    let night = || input("N", hm(18, 0), hm(6, 0));
    assert_eq!(ShiftPlan::validate(vec![]), Err(PlanRefusal::None));
    assert_eq!(
        ShiftPlan::validate(
            (0..13)
                .map(|i| input(&format!("S{i}"), hm(i, 0), hm((i + 12) % 24, 0)))
                .collect()
        ),
        Err(PlanRefusal::TooMany)
    );
    assert_eq!(
        ShiftPlan::validate(vec![day(), input("  ", hm(18, 0), hm(6, 0))]),
        Err(PlanRefusal::EmptyName(1))
    );
    assert_eq!(
        ShiftPlan::validate(vec![input(&"x".repeat(41), hm(6, 0), hm(18, 0)), night()]),
        Err(PlanRefusal::NameTooLong(0))
    );
    assert_eq!(
        ShiftPlan::validate(vec![day(), input(" a ", hm(18, 0), hm(6, 0))]),
        Err(PlanRefusal::DuplicateName(1))
    );
    assert_eq!(
        ShiftPlan::validate(vec![day(), night(), input("Z", hm(12, 0), hm(12, 0))]),
        Err(PlanRefusal::ZeroLength(2))
    );
    assert_eq!(
        ShiftPlan::validate(vec![with_break(day(), hm(20, 0), hm(21, 0)), night()]),
        Err(PlanRefusal::BreakOutsideShift(0))
    );
}

#[test]
fn p12e_a_repeated_template_id_is_refused() {
    // Review of PR 6274: A and B both name template X, so the save would update X twice and store only B and C,
    // leaving 06:00–14:00 uncovered. The list is refused at the second X.
    let with_id = |mut t: ShiftTemplateInput, id: &str| {
        t.id = Some(id.to_owned());
        t
    };
    assert_eq!(
        ShiftPlan::validate(vec![
            with_id(input("A", hm(6, 0), hm(14, 0)), "X"),
            with_id(input("B", hm(14, 0), hm(22, 0)), "X"),
            input("C", hm(22, 0), hm(6, 0)),
        ]),
        Err(PlanRefusal::DuplicateId(1))
    );
    assert_eq!(PlanRefusal::DuplicateId(1).reason(), "duplicateId");
    assert_eq!(PlanRefusal::DuplicateId(1).detail(), "1");
    // Distinct ids, and any number of new rows (no id), still pass.
    assert!(
        ShiftPlan::validate(vec![
            with_id(input("A", hm(6, 0), hm(14, 0)), "X"),
            with_id(input("B", hm(14, 0), hm(22, 0)), "Y"),
            input("C", hm(22, 0), hm(2, 0)),
            input("D", hm(2, 0), hm(6, 0)),
        ])
        .is_ok()
    );
}

#[test]
fn p12b_a_gap_is_refused_and_named() {
    // A 06–14, B 14–22: nothing covers 22:00–06:00.
    assert_eq!(
        ShiftPlan::validate(vec![
            input("A", hm(6, 0), hm(14, 0)),
            input("B", hm(14, 0), hm(22, 0)),
        ]),
        Err(PlanRefusal::Gap {
            from: hm(22, 0),
            to: hm(6, 0)
        })
    );
    // The owner's first table leaves 04:00–10:00 uncovered.
    assert_eq!(
        ShiftPlan::validate(vec![
            input("A", hm(10, 0), hm(19, 0)),
            input("B", hm(15, 0), hm(0, 0)),
            input("C", hm(19, 0), hm(4, 0)),
        ]),
        Err(PlanRefusal::Gap {
            from: hm(4, 0),
            to: hm(10, 0)
        })
    );
    // Several gaps: the first one going forward from the cutover (06:00) is named.
    assert_eq!(
        ShiftPlan::validate(vec![
            input("A", hm(6, 0), hm(10, 0)),
            input("B", hm(12, 0), hm(20, 0)),
            input("C", hm(22, 0), hm(4, 0)),
        ]),
        Err(PlanRefusal::Gap {
            from: hm(10, 0),
            to: hm(12, 0)
        })
    );
    assert_eq!(
        PlanRefusal::Gap {
            from: hm(4, 0),
            to: hm(6, 0)
        }
        .detail(),
        "04:00-06:00"
    );
}

#[test]
fn p12c_exact_and_overlapping_24_hour_plans_pass() {
    assert!(
        ShiftPlan::validate(ShiftPlan::defaults()).is_ok(),
        "exact 3 × 8 h"
    );
    assert!(
        ShiftPlan::validate(overlap24_inputs()).is_ok(),
        "overlapping"
    );
    assert!(
        ShiftPlan::validate(vec![
            input("Day", hm(8, 0), hm(20, 0)),
            input("Night", hm(20, 0), hm(8, 0)),
        ])
        .is_ok(),
        "2 × 12 h"
    );
    assert!(
        ShiftPlan::validate(vec![
            input(&"ก".repeat(40), hm(6, 0), hm(18, 0)),
            input("N", hm(18, 0), hm(6, 0)),
        ])
        .is_ok(),
        "a 40-character name"
    );
}

#[test]
fn p12d_validate_trims_names_and_keeps_order() {
    let out = ShiftPlan::validate(vec![
        input(" Night ", hm(18, 0), hm(6, 0)),
        input("Day", hm(6, 0), hm(18, 0)),
    ])
    .expect("valid");
    assert_eq!(
        out.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Night", "Day"]
    );
}

#[test]
fn p13_the_defaults_are_three_shifts_of_the_default_length_from_six() {
    assert_eq!(ShiftPlan::DEFAULT_SHIFT_HOURS, 8);
    let d = ShiftPlan::defaults();
    assert_eq!(
        d.iter()
            .map(|t| (t.name.as_str(), t.starts_at, t.ends_at, t.break_window))
            .collect::<Vec<_>>(),
        vec![
            ("A", hm(6, 0), hm(14, 0), None),
            ("B", hm(14, 0), hm(22, 0), None),
            ("C", hm(22, 0), hm(6, 0), None),
        ]
    );
    assert_eq!(ShiftPlan::validate(d.clone()), Ok(d));
}

#[test]
fn p14_the_timezone_is_the_plans() {
    let plan = overlap24_in(chrono_tz::Asia::Tokyo);
    // 10:00 in Tokyo is 08:00 in Bangkok: only A covers.
    assert_eq!(covering(&plan, bkk(4, 8, 0)), vec![named("A", 4)]);
    // 06:00 in Tokyo is 04:00 in Bangkok: the cutover.
    assert_eq!(plan.business_date_of(bkk(4, 3, 59)), Some(date(3)));
    assert_eq!(plan.business_date_of(bkk(4, 4, 0)), Some(date(4)));
}

#[test]
fn p15_an_empty_plan_covers_nothing_and_has_no_span() {
    let plan = ShiftPlan::new(vec![], Bangkok);
    assert!(covering(&plan, bkk(4, 12, 0)).is_empty());
    assert_eq!(plan.service_span(date(4)), None);
    assert_eq!(plan.span_for(bkk(4, 12, 0)), None);
    assert_eq!(plan.business_date_of(bkk(4, 12, 0)), None);
}

#[test]
fn p18_defaults_open_each_shift_at_its_start() {
    let plan = owner_defaults();
    assert_eq!(covering(&plan, bkk(4, 6, 0)), vec![named("A", 4)]);
    assert_eq!(covering(&plan, bkk(4, 14, 0)), vec![named("B", 4)]);
    assert_eq!(covering(&plan, bkk(4, 22, 0)), vec![named("C", 4)]);
}

#[test]
fn p19_defaults_date_the_night_by_the_day_it_started_and_cut_over_at_six() {
    let plan = owner_defaults();
    assert_eq!(covering(&plan, bkk(5, 2, 0)), vec![named("C", 4)]);
    assert_eq!(plan.business_date_of(bkk(5, 2, 0)), Some(date(4)));
    assert_eq!(plan.business_date_of(bkk(5, 5, 59)), Some(date(4)));
    assert_eq!(plan.business_date_of(bkk(5, 6, 0)), Some(date(5)));
}

#[test]
fn p20_defaults_run_twenty_four_hours_from_six() {
    let plan = owner_defaults();
    assert_eq!(
        plan.service_span(date(4)),
        Some((bkk(4, 6, 0), bkk(5, 6, 0)))
    );
    assert_eq!(
        plan.span_for(bkk(5, 2, 0)),
        Some((bkk(4, 6, 0), bkk(5, 6, 0)))
    );
}
