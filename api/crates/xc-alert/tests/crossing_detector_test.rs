//! AL-2 (#1002, rate alerts RA-2 of #561) — `CrossingDetector::observe`, pure (design §2 R5–R7, §4.2, §9.1 row
//! "RA-2 (`CrossingDetector`, pure)").
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Signatures this test assumes (ticket §7, `xc-alert/src/domain/crossing_detector.rs`):
//! - `CrossingDetector::observe(state: AlertState, observation: Observation, now: DateTime<Utc>)
//!   -> (AlertState, Option<Fire>)`, where `now` is the observation's time (the publication's `published_at`).
//! - `AlertState { condition, target, cooldown_seconds, relation, episode_no, last_triggered_at }` and
//!   `Fire { episode_no, observed_rate, trigger_at }`, all public, `Clone + Debug + PartialEq`.
//! - On a fire the new state's `last_triggered_at` is `now`; a crossing inside the cooldown opens its episode
//!   (`episode_no + 1`, `SATISFIED`) but leaves `last_triggered_at` as it was (R7: the cooldown counts from the last
//!   *fired* trigger).

use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use xc_alert::domain::condition::AlertCondition;
use xc_alert::domain::crossing_detector::{AlertState, CrossingDetector, Fire};
use xc_alert::domain::relation::Relation;
use xc_alert::domain::source_policy::Observation;

const DAY: i32 = 86_400;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn t(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("rfc3339")
        .with_timezone(&Utc)
}

fn now() -> DateTime<Utc> {
    t("2026-11-10T03:00:00Z")
}

/// An `ABOVE 34` alert with a 24-hour cooldown, never triggered.
fn above(relation: Relation, episode_no: i32) -> AlertState {
    AlertState {
        condition: AlertCondition::Above,
        target: dec("34.00000000"),
        cooldown_seconds: DAY,
        relation,
        episode_no,
        last_triggered_at: None,
    }
}

fn below(relation: Relation, episode_no: i32) -> AlertState {
    AlertState {
        condition: AlertCondition::Below,
        target: dec("33.00000000"),
        cooldown_seconds: DAY,
        relation,
        episode_no,
        last_triggered_at: None,
    }
}

fn satisfied_above() -> Observation {
    Observation::Compare(dec("34.10000000"))
}

fn not_satisfied_above() -> Observation {
    Observation::Compare(dec("33.90000000"))
}

// ── the relation table (R5, §4.2) ───────────────────────────────────────────

#[test]
fn unknown_then_satisfied_becomes_satisfied_without_firing() {
    let (state, fire) =
        CrossingDetector::observe(above(Relation::Unknown, 0), satisfied_above(), now());
    assert_eq!(fire, None, "the first observation never fires (R5)");
    assert_eq!(state.relation, Relation::Satisfied);
    assert_eq!(state.episode_no, 0, "no crossing, no episode");
    assert_eq!(state.last_triggered_at, None);
}

#[test]
fn unknown_then_not_satisfied_becomes_not_satisfied_without_firing() {
    let (state, fire) =
        CrossingDetector::observe(above(Relation::Unknown, 0), not_satisfied_above(), now());
    assert_eq!(fire, None);
    assert_eq!(state.relation, Relation::NotSatisfied);
    assert_eq!(state.episode_no, 0);
}

#[test]
fn not_satisfied_then_satisfied_fires_and_opens_the_next_episode() {
    let (state, fire) =
        CrossingDetector::observe(above(Relation::NotSatisfied, 3), satisfied_above(), now());
    assert_eq!(
        fire,
        Some(Fire {
            episode_no: 4,
            observed_rate: dec("34.10000000"),
            trigger_at: now(),
        }),
        "a crossing outside the cooldown fires once for episode n+1 (R5, R6)"
    );
    assert_eq!(state.relation, Relation::Satisfied);
    assert_eq!(state.episode_no, 4);
    assert_eq!(state.last_triggered_at, Some(now()));
}

#[test]
fn satisfied_then_satisfied_changes_nothing() {
    let before = above(Relation::Satisfied, 2);
    let (state, fire) = CrossingDetector::observe(before.clone(), satisfied_above(), now());
    assert_eq!(fire, None, "satisfied → satisfied is not a crossing (R5)");
    assert_eq!(state, before);
}

#[test]
fn satisfied_then_not_satisfied_ends_the_episode_without_firing() {
    let (state, fire) =
        CrossingDetector::observe(above(Relation::Satisfied, 2), not_satisfied_above(), now());
    assert_eq!(fire, None);
    assert_eq!(state.relation, Relation::NotSatisfied);
    assert_eq!(
        state.episode_no, 2,
        "the episode number stays; the next crossing opens 3 (R6)"
    );
}

#[test]
fn not_satisfied_then_not_satisfied_changes_nothing() {
    let before = above(Relation::NotSatisfied, 2);
    let (state, fire) = CrossingDetector::observe(before.clone(), not_satisfied_above(), now());
    assert_eq!(fire, None);
    assert_eq!(state, before);
}

#[test]
fn a_skipped_side_changes_no_state() {
    for relation in [
        Relation::Unknown,
        Relation::Satisfied,
        Relation::NotSatisfied,
    ] {
        let before = above(relation, 1);
        let (state, fire) = CrossingDetector::observe(before.clone(), Observation::Skip, now());
        assert_eq!(fire, None, "{relation:?}: a skipped side never fires (R8)");
        assert_eq!(
            state, before,
            "{relation:?}: a skipped side leaves the relation unchanged (R8, §4.2)"
        );
    }
}

// ── the cooldown (R7) ───────────────────────────────────────────────────────

#[test]
fn a_crossing_one_second_inside_the_cooldown_opens_the_episode_without_firing() {
    let last = t("2026-11-09T03:00:01Z");
    let at = last + Duration::seconds(i64::from(DAY)) - Duration::seconds(1);
    let state = AlertState {
        last_triggered_at: Some(last),
        ..above(Relation::NotSatisfied, 1)
    };

    let (state, fire) = CrossingDetector::observe(state, satisfied_above(), at);
    assert_eq!(
        fire, None,
        "a crossing before last_triggered_at + cooldown writes no event (R7)"
    );
    assert_eq!(state.relation, Relation::Satisfied, "the episode is open");
    assert_eq!(
        state.episode_no, 2,
        "the suppressed crossing still opens episode n+1"
    );
    assert_eq!(
        state.last_triggered_at,
        Some(last),
        "only a fire moves last_triggered_at"
    );

    let later = at + Duration::seconds(2 * i64::from(DAY));
    let (state, fire) = CrossingDetector::observe(state, satisfied_above(), later);
    assert_eq!(
        fire, None,
        "it does not fire later either: the next still-satisfied publication is not a crossing (R7)"
    );
    assert_eq!(state.episode_no, 2);
}

#[test]
fn a_crossing_at_exactly_the_end_of_the_cooldown_fires() {
    let last = t("2026-11-09T03:00:00Z");
    let at = last + Duration::seconds(i64::from(DAY));
    let state = AlertState {
        last_triggered_at: Some(last),
        ..above(Relation::NotSatisfied, 1)
    };
    let (state, fire) = CrossingDetector::observe(state, satisfied_above(), at);
    assert_eq!(
        fire.map(|f| f.episode_no),
        Some(2),
        "at last_triggered_at + cooldown the crossing fires"
    );
    assert_eq!(state.last_triggered_at, Some(at));
}

#[test]
fn a_zero_cooldown_fires_on_every_crossing() {
    let state = AlertState {
        cooldown_seconds: 0,
        last_triggered_at: Some(now()),
        ..above(Relation::NotSatisfied, 1)
    };
    let (_, fire) = CrossingDetector::observe(state, satisfied_above(), now());
    assert_eq!(fire.map(|f| f.episode_no), Some(2));
}

#[test]
fn the_first_ever_crossing_fires_whatever_the_cooldown() {
    let (_, fire) =
        CrossingDetector::observe(above(Relation::NotSatisfied, 0), satisfied_above(), now());
    assert_eq!(
        fire.map(|f| f.episode_no),
        Some(1),
        "no last_triggered_at means no cooldown is running"
    );
}

// ── rate = target (R3) ──────────────────────────────────────────────────────

#[test]
fn above_fires_at_rate_equal_to_target() {
    let (_, fire) = CrossingDetector::observe(
        above(Relation::NotSatisfied, 0),
        Observation::Compare(dec("34.00000000")),
        now(),
    );
    assert!(fire.is_some(), "ABOVE is rate >= target");
}

#[test]
fn below_fires_at_rate_equal_to_target() {
    let (_, fire) = CrossingDetector::observe(
        below(Relation::NotSatisfied, 0),
        Observation::Compare(dec("33.00000000")),
        now(),
    );
    assert!(fire.is_some(), "BELOW is rate <= target");
}

#[test]
fn below_crosses_when_the_rate_falls_to_or_under_the_target() {
    let (state, fire) = CrossingDetector::observe(
        below(Relation::Unknown, 0),
        Observation::Compare(dec("33.50000000")),
        now(),
    );
    assert_eq!(
        (state.relation, fire.is_some()),
        (Relation::NotSatisfied, false)
    );
    let (state, fire) =
        CrossingDetector::observe(state, Observation::Compare(dec("32.90000000")), now());
    assert_eq!(
        (state.relation, fire.map(|f| f.episode_no)),
        (Relation::Satisfied, Some(1))
    );
}
