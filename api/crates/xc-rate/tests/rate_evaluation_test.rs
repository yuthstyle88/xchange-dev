//! Ticket #598 (RE-1+RE-3) — `DraftEvaluation::of`: which sides of a draft are in-bound and which
//! are out-of-bound (RE-1). Written before the implementation by the ticket author
//! (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.3 (guards: the allowed range of the final rate's
//! distance from its reference, in %), §4.5 (a side is out-of-bound when it has no guard, when its
//! distance lies outside `[min_pct, max_pct]`, or when it is MANUAL with no reference; a MANUAL
//! side with a reference is measured like an AUTO side), §4.2 (the reference is normalised to the
//! currency's unit before it is used), D-6 (no policy ⇒ every line out-of-bound), §9 RE-1.
//!
//! Pure: no database. The proposals are made by `RateProposal::of`, as the service makes them.
//!
//! Signatures this test assumes, besides those listed in `rate_proposal_test.rs`:
//! - `xc_rate::domain::draft::draft_evaluation::DraftEvaluation::of(lines: &[DraftLine],
//!   proposals: &[ProposedLine], references: &[ReferenceLine], policy: Option<&RatePolicy>)
//!   -> Evaluation` — `proposals[i]` is the proposal of `lines[i]`; references are matched to
//!   lines by currency, and a currency with no reference has none in `references`.
//! - `Evaluation::is_in_bound(&self) -> bool` (no side out-of-bound) and
//!   `Evaluation::out_of_bound(&self) -> Vec<(CurrencyCode, Side)>` (every out-of-bound side, in
//!   any order, each once).
//! - `ReferenceLine` is `Clone`.
//! - The distance is `(proposed − normalised reference) ÷ normalised reference × 100`, and the
//!   guard's bounds are inclusive (the boundary cases below sit exactly on them).
//! - A fresh draft (these tests have no current publication) changes every side it prices, so
//!   "changes and has no guard" (§4.5) holds for each priced side of a currency with no guard.

use std::collections::HashSet;

use serde_json::{Value, json};
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, Side, UnitSize};
use xc_rate::domain::draft::draft_evaluation::{DraftEvaluation, Evaluation};
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_proposal::{ProposedLine, RateProposal};
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::domain::reference::reference_batch::ReferenceLine;

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known code")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a valid rate")
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).expect("a valid unit size")
}

fn pct(text: &str) -> RatePercent {
    RatePercent::parse(text).expect("a valid percent")
}

fn guard(min: &str, max: &str) -> Value {
    json!({ "min_pct": min, "max_pct": max })
}

/// Adjustments `buy`/`sell`; guards USD and JPY `[-2, 0.5]` on both sides; EUR has no entry.
fn policy_with(buy: &str, sell: &str) -> RatePolicy {
    let guards = json!({
        "USD": { "buy": guard("-2.0000", "0.5000"), "sell": guard("-2.0000", "0.5000") },
        "JPY": { "buy": guard("-2.0000", "0.5000"), "sell": guard("-2.0000", "0.5000") },
    });
    RatePolicy::new("policy-under-test", pct(buy), pct(sell), &guards).expect("a valid policy")
}

fn policy() -> RatePolicy {
    policy_with("0", "0")
}

fn units_of(currency: &str) -> CurrencyUnits {
    let size = match currency {
        "JPY" => "100",
        "KRW" => "1000",
        _ => "1",
    };
    CurrencyUnits::new(code(currency), unit(size), MinorUnit::TWO)
}

fn manual(text: &str) -> LineSide {
    LineSide::Manual(rate(text))
}

/// One draft line of `currency`, with its reference line in the provider's unit, if any.
struct Case {
    currency: &'static str,
    line: DraftLine,
    reference: Option<ReferenceLine>,
}

fn case(
    currency: &'static str,
    buy: LineSide,
    sell: LineSide,
    reference: Option<(&str, &str, &str)>,
) -> Case {
    let reason = match (&buy, &sell) {
        (LineSide::Auto, LineSide::Auto) => None,
        _ => Some("test".to_owned()),
    };
    Case {
        currency,
        line: DraftLine::new(code(currency), buy, sell, reason),
        reference: reference.map(|(ref_unit, b, s)| {
            ReferenceLine::new(code(currency), unit(ref_unit), Some(rate(b)), Some(rate(s)))
        }),
    }
}

/// Propose every line against its own reference under `policy`, then evaluate the draft.
fn evaluate(cases: &[Case], policy: Option<&RatePolicy>) -> Evaluation {
    let lines: Vec<DraftLine> = cases.iter().map(|c| c.line.clone()).collect();
    let proposals: Vec<ProposedLine> = cases
        .iter()
        .map(|c| {
            RateProposal::of(&c.line, c.reference.as_ref(), policy, &units_of(c.currency))
                .unwrap_or_else(|e| {
                    panic!("{}: the test's line is valid ({})", c.currency, e.code())
                })
        })
        .collect();
    let references: Vec<ReferenceLine> = cases.iter().filter_map(|c| c.reference.clone()).collect();
    DraftEvaluation::of(&lines, &proposals, &references, policy)
}

fn out(e: &Evaluation) -> HashSet<(&'static str, Side)> {
    let list = e.out_of_bound();
    let set: HashSet<(&'static str, Side)> = list.iter().map(|(c, s)| (c.as_str(), *s)).collect();
    assert_eq!(
        set.len(),
        list.len(),
        "each out-of-bound side is listed once: {list:?}"
    );
    assert_eq!(
        e.is_in_bound(),
        set.is_empty(),
        "is_in_bound agrees with out_of_bound"
    );
    set
}

fn set(items: &[(&'static str, Side)]) -> HashSet<(&'static str, Side)> {
    items.iter().copied().collect()
}

const USD_REF: (&str, &str, &str) = ("1", "40", "40");

// ── MANUAL with and without a reference ─────────────────────────────────────────

#[test]
fn a_manual_side_with_a_reference_inside_the_guard_is_in_bound() {
    // 40.1 is +0.25% of 40, inside [-2, 0.5]; 39.5 is −1.25%
    let e = evaluate(
        &[case("USD", manual("39.5"), manual("40.1"), Some(USD_REF))],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[]));
    assert!(e.is_in_bound());
}

#[test]
fn a_manual_side_with_no_reference_is_out_of_bound_even_when_its_currency_has_a_guard() {
    let e = evaluate(
        &[case("USD", manual("39.5"), manual("40.1"), None)],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[("USD", Side::Buy), ("USD", Side::Sell)]));
    assert!(!e.is_in_bound());
}

#[test]
fn a_manual_side_outside_the_guard_is_out_of_bound_and_only_that_side() {
    // sell 40.4 is +1% of 40, above 0.5; buy 39.5 is inside
    let e = evaluate(
        &[case("USD", manual("39.5"), manual("40.4"), Some(USD_REF))],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[("USD", Side::Sell)]));
}

// ── the guard's bounds are inclusive ───────────────────────────────────────────

#[test]
fn a_side_exactly_on_a_bound_is_in_bound_and_one_unit_beyond_is_out() {
    // 40.2 = +0.5% exactly; 39.2 = −2% exactly
    let on = evaluate(
        &[case("USD", manual("39.2"), manual("40.2"), Some(USD_REF))],
        Some(&policy()),
    );
    assert_eq!(out(&on), set(&[]), "both sides on their bounds");

    let beyond = evaluate(
        &[case(
            "USD",
            manual("39.19999999"),
            manual("40.20000001"),
            Some(USD_REF),
        )],
        Some(&policy()),
    );
    assert_eq!(
        out(&beyond),
        set(&[("USD", Side::Buy), ("USD", Side::Sell)]),
        "one unit of the 8th place beyond each bound"
    );
}

// ── AUTO is measured the same way ──────────────────────────────────────────────

#[test]
fn an_auto_side_is_measured_by_its_adjustment_against_the_guard() {
    let auto = || case("USD", LineSide::Auto, LineSide::Auto, Some(USD_REF));
    // +0.3% on both sides: inside [-2, 0.5]
    assert_eq!(
        out(&evaluate(&[auto()], Some(&policy_with("0.3", "0.3")))),
        set(&[])
    );
    // sell +0.6%: outside; buy −1%: inside
    assert_eq!(
        out(&evaluate(&[auto()], Some(&policy_with("-1", "0.6")))),
        set(&[("USD", Side::Sell)])
    );
}

#[test]
fn a_currency_with_no_guard_is_out_of_bound_on_every_side_it_prices() {
    // EUR has no guard entry; USD (in-bound) shows that only EUR's sides are listed
    let e = evaluate(
        &[
            case(
                "EUR",
                LineSide::Auto,
                LineSide::Auto,
                Some(("1", "37.4", "38.0")),
            ),
            case("USD", LineSide::Auto, LineSide::Auto, Some(USD_REF)),
        ],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[("EUR", Side::Buy), ("EUR", Side::Sell)]));
}

// ── the reference is normalised before it is measured ──────────────────────────

#[test]
fn a_jpy_side_is_measured_against_its_reference_normalised_to_per_100() {
    // reference per 1: 0.235 → 23.5 per 100. 23.6 per 100 is +0.4255…%, inside [-2, 0.5].
    // Measured against the raw 0.235 it would be ~+9,900% and out-of-bound.
    let e = evaluate(
        &[case(
            "JPY",
            manual("23.2"),
            manual("23.6"),
            Some(("1", "0.235", "0.235")),
        )],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[]), "per-1 reference, per-100 rate");

    // the same reference quoted per 100 (as BoT quotes it): measured as is
    let e = evaluate(
        &[case(
            "JPY",
            manual("23.2"),
            manual("23.6"),
            Some(("100", "23.5", "23.5")),
        )],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[]), "per-100 reference, per-100 rate");

    // and 23.7 per 100 is +0.85…%: out on the sell side
    let e = evaluate(
        &[case(
            "JPY",
            manual("23.2"),
            manual("23.7"),
            Some(("100", "23.5", "23.5")),
        )],
        Some(&policy()),
    );
    assert_eq!(out(&e), set(&[("JPY", Side::Sell)]));
}

// ── no policy ──────────────────────────────────────────────────────────────────

#[test]
fn under_no_policy_every_priced_side_is_out_of_bound() {
    // D-6: no policy, no AUTO side, so a draft prices only by MANUAL, and nothing is in-bound
    let e = evaluate(
        &[case("USD", manual("39.5"), manual("40.1"), Some(USD_REF))],
        None,
    );
    assert_eq!(out(&e), set(&[("USD", Side::Buy), ("USD", Side::Sell)]));
}

#[test]
fn evaluating_twice_gives_the_same_answer() {
    let cases = [
        case("USD", manual("39.5"), manual("40.4"), Some(USD_REF)),
        case(
            "EUR",
            LineSide::Auto,
            LineSide::Auto,
            Some(("1", "37.4", "38.0")),
        ),
    ];
    let a = out(&evaluate(&cases, Some(&policy())));
    let b = out(&evaluate(&cases, Some(&policy())));
    assert_eq!(a, b);
    assert_eq!(
        a,
        set(&[("USD", Side::Sell), ("EUR", Side::Buy), ("EUR", Side::Sell)])
    );
}
