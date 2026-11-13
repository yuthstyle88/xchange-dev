//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §6a): `DraftEvaluation::of_tiered_with` checks
//! the guard on the tier prices the resolved formula gives, not on the default formula's.
//!
//! Written before the implementation. Do not edit, delete or weaken. Pure: no database.

mod pricing_formula_shared;

use std::collections::HashMap;

use pricing_formula_shared::*;
use xc_kernel::money::Side;
use xc_rate::domain::draft::draft_evaluation::DraftEvaluation;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_proposal::RateProposal;
use xc_rate::domain::formula::price_formula::PriceFormula;

#[test]
fn the_default_formula_answers_exactly_what_of_tiered_answers() {
    let line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);
    let usd_units = HashMap::from([(code("USD"), units("USD", "1"))]);
    for p in [
        policy_guarded("0", "0", "-1.0000", "1.0000"),
        policy_guarded("0", "0", "-0.1500", "0.1500"),
    ] {
        let prop = RateProposal::of(&line, Some(&usd_1028()), Some(&p), &units("USD", "1"))
            .expect("proposal");
        let lines = vec![line.clone()];
        let props = vec![prop];
        let refs = vec![usd_1028()];
        let tiers = vec![Some(example_tiers())];
        let today = DraftEvaluation::of_tiered(&lines, &props, &refs, Some(&p), &tiers, &usd_units);
        let with = DraftEvaluation::of_tiered_with(
            &lines,
            &props,
            &refs,
            Some(&p),
            &tiers,
            &[PriceFormula::default_formula()],
            &usd_units,
        );
        assert_eq!(with, today);
    }
}

#[test]
fn the_guard_sees_the_resolved_formulas_tier_prices() {
    // guard ±1 %: the default tiers (margins up to ±0.3 %) are in bound; a formula 2 % above the reference is not
    let p = policy_guarded("0", "0", "-1.0000", "1.0000");
    let line = DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None);
    let usd_units = HashMap::from([(code("USD"), units("USD", "1"))]);
    let prop =
        RateProposal::of(&line, Some(&usd_1028()), Some(&p), &units("USD", "1")).expect("proposal");
    let lines = vec![line];
    let props = vec![prop];
    let refs = vec![usd_1028()];
    let tiers = vec![Some(example_tiers())];

    assert!(
        DraftEvaluation::of_tiered(&lines, &props, &refs, Some(&p), &tiers, &usd_units)
            .is_in_bound()
    );

    let up_2pct = formula("ref_buy * 1.02", "ref_sell * 1.02");
    let e = DraftEvaluation::of_tiered_with(
        &lines,
        &props,
        &refs,
        Some(&p),
        &tiers,
        &[up_2pct],
        &usd_units,
    );
    assert!(!e.is_in_bound());
    let out = e.out_of_bound();
    assert!(out.contains(&(code("USD"), Side::Buy)), "{out:?}");
    assert!(out.contains(&(code("USD"), Side::Sell)), "{out:?}");
}
