//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §2, §6a; B1, B3, B16, B26):
//! `RateProposal::of_with`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Pure: no database.

mod pricing_formula_shared;

use pricing_formula_shared::*;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_proposal::RateProposal;
use xc_rate::domain::formula::price_formula::PriceFormula;

fn auto(c: &str) -> DraftLine {
    DraftLine::new(code(c), LineSide::Auto, LineSide::Auto, None)
}

#[test]
fn b1_the_default_formula_answers_exactly_what_of_answers() {
    let cases = [
        ("USD", "1", usd_1028(), policy("0.3000", "-0.3000")),
        ("USD", "1", usd_1028(), policy("0", "0")),
        (
            "USD",
            "1",
            reference("USD", "1", Some("33.12345678"), Some("33.98765432")),
            policy("-1.2345", "2.3456"),
        ),
        (
            "JPY",
            "100",
            reference("JPY", "1", Some("0.215432"), Some("0.221987")),
            policy("0.3000", "-0.3000"),
        ),
        (
            "USD",
            "1",
            reference("USD", "1", Some("33.6482"), None),
            policy("0.3000", "-0.3000"),
        ),
    ];
    let default = PriceFormula::default_formula();
    for (c, size, r, p) in cases {
        let u = units(c, size);
        let line = auto(c);
        let today = RateProposal::of(&line, Some(&r), Some(&p), &u).expect("of");
        let with = RateProposal::of_with(&line, Some(&r), Some(&p), &u, &default).expect("of_with");
        assert_eq!(with, today, "{c} {:?}/{:?}", r.buy, r.sell);
    }
}

#[test]
fn b1_no_policy_or_no_reference_still_gives_no_auto_price() {
    let default = PriceFormula::default_formula();
    let u = units("USD", "1");
    let none = RateProposal::of_with(&auto("USD"), None, Some(&policy("0", "0")), &u, &default)
        .expect("no reference");
    assert_eq!((none.buy, none.sell), (None, None));
    let no_policy =
        RateProposal::of_with(&auto("USD"), Some(&usd_1028()), None, &u, &mid_formula())
            .expect("no policy");
    assert_eq!((no_policy.buy, no_policy.sell), (None, None));
}

#[test]
fn b3_an_organisation_formula_prices_the_auto_sides() {
    let p = RateProposal::of_with(
        &auto("USD"),
        Some(&usd_1028()),
        Some(&policy("0.3000", "-0.3000")),
        &units("USD", "1"),
        &mid_formula(),
    )
    .expect("priced");
    assert_eq!(p.buy, Some(rate("33.75765")));
    assert_eq!(p.sell, Some(rate("33.85765")));
}

#[test]
fn b26_constants_are_per_the_currencys_unit() {
    // JPY: unit size 100, the reference quoted per 1 JPY. ref_* at the unit size: 21.5432 / 22.1987, mid 21.87095.
    let p = RateProposal::of_with(
        &auto("JPY"),
        Some(&reference("JPY", "1", Some("0.215432"), Some("0.221987"))),
        Some(&policy("0", "0")),
        &units("JPY", "100"),
        &mid_formula(),
    )
    .expect("priced");
    assert_eq!(
        p.buy,
        Some(rate("21.82095")),
        "0.05 THB below the mid per 100 JPY"
    );
    assert_eq!(
        p.sell,
        Some(rate("21.92095")),
        "0.05 THB above the mid per 100 JPY"
    );
}

#[test]
fn a_manual_side_is_kept_and_only_the_auto_side_is_priced() {
    let line = DraftLine::new(
        code("USD"),
        LineSide::Manual(rate("33.10")),
        LineSide::Auto,
        Some("desk call".into()),
    );
    let p = RateProposal::of_with(
        &line,
        Some(&usd_1028()),
        Some(&policy("0", "0")),
        &units("USD", "1"),
        &mid_formula(),
    )
    .expect("priced");
    assert_eq!(p.buy, Some(rate("33.10")));
    assert_eq!(p.sell, Some(rate("33.85765")));
}

#[test]
fn a_formula_naming_a_missing_reference_side_leaves_that_side_unpriced() {
    // a buy-only reference line: ref_mid has no value, so both mid-based sides are None, as a missing side is today
    let buy_only = reference("USD", "1", Some("33.6482"), None);
    let p = RateProposal::of_with(
        &auto("USD"),
        Some(&buy_only),
        Some(&policy("0", "0")),
        &units("USD", "1"),
        &mid_formula(),
    )
    .expect("no error");
    assert_eq!((p.buy, p.sell), (None, None));
    let q = RateProposal::of_with(
        &auto("USD"),
        Some(&buy_only),
        Some(&policy("0", "0")),
        &units("USD", "1"),
        &formula("ref_buy - 0.05", "ref_sell + 0.05"),
    )
    .expect("no error");
    assert_eq!((q.buy, q.sell), (Some(rate("33.5982")), None));
}

#[test]
fn any_other_formula_error_is_validation_failed() {
    for f in [
        formula("ref_buy / (buy_pct - buy_pct)", "ref_sell"),
        formula("ref_buy * 0", "ref_sell"),
        formula("ref_buy", "ref_sell - 40"),
    ] {
        let e = RateProposal::of_with(
            &auto("USD"),
            Some(&usd_1028()),
            Some(&policy("0", "0")),
            &units("USD", "1"),
            &f,
        )
        .expect_err("refused");
        assert_eq!(
            e.code(),
            "validationFailed",
            "{} / {}: {e}",
            f.buy.as_str(),
            f.sell.as_str()
        );
    }
}

#[test]
fn b16_buy_above_sell_is_still_refused() {
    let e = RateProposal::of_with(
        &auto("USD"),
        Some(&usd_1028()),
        Some(&policy("0", "0")),
        &units("USD", "1"),
        &formula("ref_mid + 0.10", "ref_mid - 0.10"),
    )
    .expect_err("inverted");
    assert_eq!(e.code(), "validationFailed");
    assert!(
        e.to_string()
            .contains("buy rate must be less than or equal to sell rate"),
        "{e}"
    );
}
