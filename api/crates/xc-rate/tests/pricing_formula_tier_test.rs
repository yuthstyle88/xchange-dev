//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §2, §6a; B1, B14): `TierProposal::of_with`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Pure: no database.

mod pricing_formula_shared;

use pricing_formula_shared::*;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_proposal::RateProposal;
use xc_rate::domain::draft::tier_proposal::TierProposal;
use xc_rate::domain::formula::price_formula::PriceFormula;

fn auto(c: &str) -> DraftLine {
    DraftLine::new(code(c), LineSide::Auto, LineSide::Auto, None)
}

#[test]
fn b1_the_default_formula_answers_exactly_what_of_answers() {
    for (c, size, r, p) in [
        ("USD", "1", usd_1028(), policy("0.1000", "-0.1000")),
        ("USD", "1", reference("USD", "1", Some("32.4500"), Some("32.9000")), policy("-0.5000", "+0.4000")),
        ("JPY", "100", reference("JPY", "1", Some("0.215432"), Some("0.221987")), policy("0", "0")),
    ] {
        let u = units(c, size);
        let line = auto(c);
        let prop = RateProposal::of(&line, Some(&r), Some(&p), &u).expect("proposal");
        let today = TierProposal::of(&prop, &line, Some(&r), Some(&p), &example_tiers(), &u).expect("of");
        let with = TierProposal::of_with(
            &prop,
            &line,
            Some(&r),
            Some(&p),
            &example_tiers(),
            &u,
            &PriceFormula::default_formula(),
        )
        .expect("of_with");
        assert_eq!(with, today, "{c}");
    }
}

#[test]
fn a_formula_sees_each_tiers_margin_as_tier_buy_pct_and_tier_sell_pct() {
    // buy = ref_buy × (1 + tier_buy_pct / 100), ignoring the policy's percentages
    let f = formula("ref_buy * (1 + tier_buy_pct / 100)", "ref_sell * (1 + tier_sell_pct / 100)");
    let r = usd_1028();
    let p = policy("0.3000", "-0.3000");
    let u = units("USD", "1");
    let line = auto("USD");
    let prop = RateProposal::of_with(&line, Some(&r), Some(&p), &u, &f).expect("proposal");
    let tiers = TierProposal::of_with(&prop, &line, Some(&r), Some(&p), &example_tiers(), &u, &f)
        .expect("tiers");
    let got: Vec<_> = tiers.iter().map(|t| (t.buy, t.sell)).collect();
    assert_eq!(
        got,
        vec![
            (Some(rate("33.6482")), Some(rate("33.9671"))),
            (Some(rate("33.6818482")), Some(rate("33.9331329"))),
            (Some(rate("33.7154964")), Some(rate("33.8991658"))),
            (Some(rate("33.7491446")), Some(rate("33.8651987"))),
        ]
    );
    // base_* are the proposal's, as today
    assert!(tiers.iter().all(|t| t.base_buy == prop.buy && t.base_sell == prop.sell));
}

#[test]
fn b14_a_tier_that_inverts_is_refused_and_named() {
    // buy = mid − 0.05; sell = mid + 0.05 + 0.6 × tier_sell_pct: T2 (−0.1) gives mid − 0.01 ≥ buy, T3 (−0.2) gives
    // mid − 0.07 < buy, the first tier that inverts
    let f = formula("ref_mid - 0.05", "ref_mid + 0.05 + tier_sell_pct * 0.6");
    let r = usd_1028();
    let p = policy("0", "0");
    let u = units("USD", "1");
    let line = auto("USD");
    let prop = RateProposal::of_with(&line, Some(&r), Some(&p), &u, &f).expect("the single line is fine");
    let e = TierProposal::of_with(&prop, &line, Some(&r), Some(&p), &example_tiers(), &u, &f)
        .expect_err("tier 3 inverts");
    assert_eq!(e.code(), "validationFailed");
    assert!(e.to_string().contains("tier 3"), "{e}");
}
