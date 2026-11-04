//! Up-front tests for ticket CS-1: `ShiftPlan` (docs/design/configurable-shifts.md §4, rows P1–P14).
//! The implementer must not edit, delete or weaken these.

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

fn defaults_in(tz: Tz) -> ShiftPlan {
    ShiftPlan::new(templates_from(ShiftPlan::defaults()), tz)
}

fn defaults() -> ShiftPlan {
    defaults_in(Bangkok)
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

fn input(name: &str, start: NaiveTime, end: NaiveTime) -> ShiftTemplateInput {
    ShiftTemplateInput {
        id: None,
        name: name.to_owned(),
        starts_at: start,
        ends_at: end,
        break_window: None,
    }
}

#[test]
fn p1_ten_oclock_is_shift_a_only() {
    assert_eq!(covering(&defaults(), bkk(4, 10, 0)), vec![named("A", 4)]);
}

#[test]
fn p2_fifteen_oclock_is_a_and_b() {
    assert_eq!(
        covering(&defaults(), bkk(4, 15, 0)),
        vec![named("A", 4), named("B", 4)]
    );
}

#[test]
fn p3_nineteen_oclock_is_b_and_c_because_a_ends_exclusive() {
    assert_eq!(
        covering(&defaults(), bkk(4, 19, 0)),
        vec![named("B", 4), named("C", 4)]
    );
}

#[test]
fn p4_two_am_is_shift_c_of_the_previous_business_date() {
    assert_eq!(covering(&defaults(), bkk(5, 2, 0)), vec![named("C", 4)]);
}

#[test]
fn p5_five_am_is_outside_every_shift() {
    assert!(covering(&defaults(), bkk(5, 5, 0)).is_empty());
}

#[test]
fn p6_one_minute_before_a_is_outside_every_shift() {
    assert!(covering(&defaults(), bkk(4, 9, 59)).is_empty());
}

#[test]
fn p7_business_date_of_two_am_is_the_day_before_and_five_am_has_none() {
    let plan = defaults();
    assert_eq!(plan.business_date_of(bkk(5, 2, 0)), Some(date(4)));
    assert_eq!(plan.business_date_of(bkk(4, 10, 0)), Some(date(4)));
    assert_eq!(plan.business_date_of(bkk(5, 5, 0)), None);
}

#[test]
fn p8_service_span_runs_ten_to_four_next_day() {
    let span = defaults().service_span(date(4));
    assert_eq!(span, Some((bkk(4, 10, 0), bkk(5, 4, 0))));
    assert_eq!(
        span.map(|(a, b)| (a.to_rfc3339(), b.to_rfc3339())),
        Some((
            "2026-11-04T03:00:00+00:00".to_owned(),
            "2026-11-04T21:00:00+00:00".to_owned()
        ))
    );
}

#[test]
fn p9_span_for_an_instant_between_spans_is_the_next_span() {
    let plan = defaults();
    assert_eq!(
        plan.span_for(bkk(5, 5, 0)),
        Some((bkk(5, 10, 0), bkk(6, 4, 0)))
    );
    assert_eq!(
        plan.span_for(bkk(5, 2, 0)),
        Some((bkk(4, 10, 0), bkk(5, 4, 0)))
    );
}

#[test]
fn p10_on_break_follows_each_templates_break() {
    let plan = defaults();
    assert!(plan.on_break("B", bkk(4, 18, 30)));
    assert!(!plan.on_break("B", bkk(4, 19, 0)));
    assert!(plan.on_break("C", bkk(4, 23, 30)));
    assert!(!plan.on_break("A", bkk(4, 18, 30)));
    assert!(plan.on_break("A", bkk(4, 15, 0)));
}

#[test]
fn p11_instance_of_c_crosses_midnight_with_its_break() {
    let plan = defaults();
    let c = plan.templates()[2].clone();
    let i = plan.instance(&c, date(4));
    assert_eq!(i.template_id, "C");
    assert_eq!(i.name, "C");
    assert_eq!(i.business_date, date(4));
    assert_eq!(i.starts_at, bkk(4, 19, 0));
    assert_eq!(i.ends_at, bkk(5, 4, 0));
    assert_eq!(i.break_window, Some((bkk(4, 23, 0), bkk(5, 0, 0))));
}

#[test]
fn p12_validate_refuses_each_broken_list() {
    let a = || input("A", hm(10, 0), hm(19, 0));
    assert_eq!(ShiftPlan::validate(vec![]), Err(PlanRefusal::None));
    assert_eq!(
        ShiftPlan::validate((0..13).map(|i| input(&format!("S{i}"), hm(10, 0), hm(11, 0))).collect()),
        Err(PlanRefusal::TooMany)
    );
    assert_eq!(
        ShiftPlan::validate(vec![a(), input("   ", hm(11, 0), hm(12, 0))]),
        Err(PlanRefusal::EmptyName(1))
    );
    assert_eq!(
        ShiftPlan::validate(vec![input(&"x".repeat(41), hm(10, 0), hm(11, 0))]),
        Err(PlanRefusal::NameTooLong(0))
    );
    assert_eq!(
        ShiftPlan::validate(vec![a(), input(" a ", hm(11, 0), hm(12, 0))]),
        Err(PlanRefusal::DuplicateName(1))
    );
    assert_eq!(
        ShiftPlan::validate(vec![a(), input("Z", hm(12, 0), hm(12, 0))]),
        Err(PlanRefusal::ZeroLength(1))
    );
    let mut with_bad_break = a();
    with_bad_break.break_window = Some((hm(5, 0), hm(6, 0)));
    assert_eq!(
        ShiftPlan::validate(vec![with_bad_break]),
        Err(PlanRefusal::BreakOutsideShift(0))
    );
    // No uncovered gap: the anchor is the earliest start (08:00) and B ends at 09:00 the next day, 25 h later.
    assert_eq!(
        ShiftPlan::validate(vec![
            input("A", hm(8, 0), hm(20, 0)),
            input("B", hm(20, 0), hm(9, 0)),
        ]),
        Err(PlanRefusal::SpanOver24h)
    );
}

#[test]
fn p12b_validate_trims_names_and_keeps_order() {
    let out = ShiftPlan::validate(vec![
        input(" Night ", hm(19, 0), hm(4, 0)),
        input("Day", hm(10, 0), hm(19, 0)),
    ])
    .expect("valid");
    assert_eq!(
        out.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Night", "Day"]
    );
}

#[test]
fn p12c_a_name_of_exactly_forty_characters_is_accepted() {
    assert!(ShiftPlan::validate(vec![input(&"ก".repeat(40), hm(10, 0), hm(11, 0))]).is_ok());
}

#[test]
fn p12d_a_plan_covering_exactly_twenty_four_hours_is_accepted() {
    assert!(
        ShiftPlan::validate(vec![
            input("Day", hm(8, 0), hm(20, 0)),
            input("Night", hm(20, 0), hm(8, 0)),
        ])
        .is_ok()
    );
}

#[test]
fn p13_the_defaults_are_valid_and_unchanged() {
    assert_eq!(ShiftPlan::validate(ShiftPlan::defaults()), Ok(ShiftPlan::defaults()));
}

#[test]
fn p14_a_plan_whose_only_shift_starts_after_midnight_dates_by_its_own_start() {
    let plan = ShiftPlan::new(
        templates_from(vec![input("Early", hm(2, 0), hm(8, 0))]),
        Bangkok,
    );
    assert_eq!(plan.business_date_of(bkk(3, 3, 0)), Some(date(3)));
    assert_eq!(plan.business_date_of(bkk(3, 9, 0)), None);
    assert_eq!(
        covering(&plan, bkk(3, 7, 59)),
        vec![("Early".to_owned(), date(3))]
    );
}

#[test]
fn p15_a_template_starting_after_midnight_inside_the_night_belongs_to_the_day_before() {
    // A 10–19, C 19–04, D 00:30–06:00: the longest uncovered gap is 06:00–10:00, so the anchor is 10:00 and
    // D's instance on 4 Nov runs 5 Nov 00:30–06:00.
    let plan = ShiftPlan::new(
        templates_from(vec![
            input("A", hm(10, 0), hm(19, 0)),
            input("C", hm(19, 0), hm(4, 0)),
            input("D", hm(0, 30), hm(6, 0)),
        ]),
        Bangkok,
    );
    assert_eq!(
        covering(&plan, bkk(5, 1, 0)),
        vec![("C".to_owned(), date(4)), ("D".to_owned(), date(4))]
    );
    assert_eq!(plan.business_date_of(bkk(5, 5, 0)), Some(date(4)));
    assert_eq!(plan.service_span(date(4)), Some((bkk(4, 10, 0), bkk(5, 6, 0))));
    assert!(ShiftPlan::validate(vec![
        input("A", hm(10, 0), hm(19, 0)),
        input("C", hm(19, 0), hm(4, 0)),
        input("D", hm(0, 30), hm(6, 0)),
    ])
    .is_ok());
}

#[test]
fn p16_the_timezone_is_the_plans() {
    let plan = defaults_in(chrono_tz::Asia::Tokyo);
    // 10:00 in Tokyo is 08:00 in Bangkok.
    assert_eq!(covering(&plan, bkk(4, 8, 0)), vec![named("A", 4)]);
    assert!(covering(&plan, bkk(4, 10, 30)).iter().any(|c| c.0 == "A"));
    assert!(covering(&plan, bkk(4, 7, 59)).is_empty());
}

#[test]
fn p17_an_empty_plan_covers_nothing_and_has_no_span() {
    let plan = ShiftPlan::new(vec![], Bangkok);
    assert!(covering(&plan, bkk(4, 12, 0)).is_empty());
    assert_eq!(plan.service_span(date(4)), None);
    assert_eq!(plan.span_for(bkk(4, 12, 0)), None);
    assert_eq!(plan.business_date_of(bkk(4, 12, 0)), None);
}
