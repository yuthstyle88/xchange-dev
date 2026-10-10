//! AL-2 (#1002, rate alerts RA-2 of #561) — the pure values: `AlertCondition` (R3), `Relation`, `ObservePolicy` and
//! `AlertSourcePolicy` (R8, Q-R3), `CustomerSide::to_shop` (R1), and the refusal codes.
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Signatures this test assumes (ticket §7):
//! - `AlertCondition::satisfied(self, rate: Decimal, target: Decimal) -> bool`; `as_str` / `parse` on
//!   `AlertCondition`, `Relation`, `ObservePolicy` use exactly the values the AL-1 CHECKs allow.
//! - `AlertSourcePolicy::new(ObservePolicy)` and `observes(&self, &SideSnapshot, &Freshness) -> Observation`,
//!   with `SideSnapshot { rate: Decimal, mode: SideMode }` and `xc_rate`'s `Freshness { stale, source_date }`.
//! - `xc_alert::domain::CustomerSide` is the customer side (the local stub until #558 APP-2 merges, then
//!   `xc_kernel`'s); `to_shop(self) -> Side`.
//! - `RateAlertError::code()` / `reason()`.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_alert::domain::CustomerSide;
use xc_alert::domain::condition::AlertCondition;
use xc_alert::domain::error::RateAlertError;
use xc_alert::domain::relation::Relation;
use xc_alert::domain::source_policy::{
    AlertSourcePolicy, Observation, ObservePolicy, SideMode, SideSnapshot,
};
use xc_kernel::money::Side;
use xc_rate::domain::publication::freshness::Freshness;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

const FRESH: Freshness = Freshness {
    stale: false,
    source_date: None,
};
const STALE: Freshness = Freshness {
    stale: true,
    source_date: None,
};

fn side(mode: SideMode) -> SideSnapshot {
    SideSnapshot {
        rate: dec("34.12345678"),
        mode,
    }
}

// ── R3 ──────────────────────────────────────────────────────────────────────

#[test]
fn above_is_rate_at_or_over_the_target() {
    let target = dec("34.00000000");
    assert!(AlertCondition::Above.satisfied(dec("34.00000001"), target));
    assert!(
        AlertCondition::Above.satisfied(dec("34"), target),
        "rate = target satisfies ABOVE"
    );
    assert!(!AlertCondition::Above.satisfied(dec("33.99999999"), target));
}

#[test]
fn below_is_rate_at_or_under_the_target() {
    let target = dec("34.00000000");
    assert!(AlertCondition::Below.satisfied(dec("33.99999999"), target));
    assert!(
        AlertCondition::Below.satisfied(dec("34.0"), target),
        "rate = target satisfies BELOW"
    );
    assert!(!AlertCondition::Below.satisfied(dec("34.00000001"), target));
}

// ── stored values (AL-1's CHECKs) ───────────────────────────────────────────

#[test]
fn conditions_relations_and_policies_use_the_stored_words() {
    for (c, s) in [
        (AlertCondition::Above, "ABOVE"),
        (AlertCondition::Below, "BELOW"),
    ] {
        assert_eq!(c.as_str(), s);
        assert_eq!(AlertCondition::parse(s), Some(c));
    }
    for (r, s) in [
        (Relation::Unknown, "UNKNOWN"),
        (Relation::Satisfied, "SATISFIED"),
        (Relation::NotSatisfied, "NOT_SATISFIED"),
    ] {
        assert_eq!(r.as_str(), s);
        assert_eq!(Relation::parse(s), Some(r));
    }
    for (p, s) in [
        (ObservePolicy::AllFresh, "ALL_FRESH"),
        (ObservePolicy::All, "ALL"),
        (ObservePolicy::AutoFresh, "AUTO_FRESH"),
    ] {
        assert_eq!(p.as_str(), s);
        assert_eq!(ObservePolicy::parse(s), Some(p));
    }
}

#[test]
fn other_spellings_are_not_read() {
    assert_eq!(AlertCondition::parse("above"), None);
    assert_eq!(AlertCondition::parse("OVER"), None);
    assert_eq!(Relation::parse("satisfied"), None);
    assert_eq!(ObservePolicy::parse("all_fresh"), None);
}

// ── R1 ──────────────────────────────────────────────────────────────────────

#[test]
fn the_customer_side_becomes_the_opposite_shop_side() {
    assert_eq!(
        CustomerSide::CustomerBuy.to_shop(),
        Side::Sell,
        "the customer buys what the shop sells"
    );
    assert_eq!(CustomerSide::CustomerSell.to_shop(), Side::Buy);
}

// ── R8, Q-R3 ────────────────────────────────────────────────────────────────

#[test]
fn all_fresh_skips_a_stale_side() {
    let policy = AlertSourcePolicy::new(ObservePolicy::AllFresh);
    assert_eq!(
        policy.observes(&side(SideMode::Auto), &STALE),
        Observation::Skip
    );
    assert_eq!(
        policy.observes(&side(SideMode::Manual), &STALE),
        Observation::Skip
    );
}

#[test]
fn all_fresh_compares_a_fresh_manual_side_like_a_fresh_auto_side() {
    let policy = AlertSourcePolicy::new(ObservePolicy::AllFresh);
    let rate = Observation::Compare(dec("34.12345678"));
    assert_eq!(policy.observes(&side(SideMode::Auto), &FRESH), rate);
    assert_eq!(
        policy.observes(&side(SideMode::Manual), &FRESH),
        rate,
        "Q-R3: a manual price is the shop's price too"
    );
}

#[test]
fn all_compares_every_side_stale_included() {
    let policy = AlertSourcePolicy::new(ObservePolicy::All);
    let rate = Observation::Compare(dec("34.12345678"));
    assert_eq!(policy.observes(&side(SideMode::Auto), &STALE), rate);
    assert_eq!(policy.observes(&side(SideMode::Manual), &STALE), rate);
}

#[test]
fn auto_fresh_compares_only_fresh_auto_sides() {
    let policy = AlertSourcePolicy::new(ObservePolicy::AutoFresh);
    assert_eq!(
        policy.observes(&side(SideMode::Auto), &FRESH),
        Observation::Compare(dec("34.12345678"))
    );
    assert_eq!(
        policy.observes(&side(SideMode::Auto), &STALE),
        Observation::Skip
    );
    assert_eq!(
        policy.observes(&side(SideMode::Manual), &FRESH),
        Observation::Skip
    );
}

// ── codes ───────────────────────────────────────────────────────────────────

#[test]
fn refusals_carry_the_v3_codes() {
    let missing = RateAlertError::ValidationFailed {
        reason: Some("alertSettingsMissing"),
        message: "no settings".into(),
    };
    assert_eq!(
        (missing.code(), missing.reason()),
        ("validationFailed", Some("alertSettingsMissing"))
    );
    let plain = RateAlertError::ValidationFailed {
        reason: None,
        message: "target".into(),
    };
    assert_eq!((plain.code(), plain.reason()), ("validationFailed", None));
    assert_eq!(RateAlertError::OutOfScope("x".into()).code(), "outOfScope");
    assert_eq!(
        RateAlertError::StaleVersion("x".into()).code(),
        "staleVersion"
    );
    assert_eq!(
        RateAlertError::IdempotencyConflict("x".into()).code(),
        "idempotencyConflict"
    );
    assert_eq!(RateAlertError::OutOfScope("x".into()).reason(), None);
}
