//! Written before the implementation (ticket T2). Do not edit, delete or weaken.
//!
//! `fixtures/v2_quote_cases.json` is what the v2 engine stored for each input
//! (see `api/tools/v2-fixtures/`). `exact` is the same figure computed in exact
//! decimal arithmetic; the generator reports how often v2's float result
//! differs from it. The Rust code is held to `exact`.

use serde::Deserialize;
use xc_domain::money::{Money, NumberError, Rate};
use xc_domain::quote::{Quote, QuoteError, Side};

#[derive(Deserialize)]
struct Figures {
    principal: String,
    received: String,
    paid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    r#type: String,
    amount: String,
    rate: String,
    fee: String,
    error: Option<String>,
    exact: Option<Figures>,
    v2: Option<Figures>,
    v2_differs_from_exact: Option<bool>,
}

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/v2_quote_cases.json")).expect("fixture parses")
}

/// The whole path a screen's numbers take, reduced to the fixture's vocabulary.
fn outcome(case: &Case) -> Result<Quote, &'static str> {
    let number = |e: NumberError| match e {
        NumberError::Invalid => "INVALID_NUMBER",
        NumberError::TooManyDecimals => "TOO_MANY_DECIMALS",
    };
    let side = if case.r#type == "BUY" {
        Side::Buy
    } else {
        Side::Sell
    };
    let amount = Money::parse(&case.amount).map_err(number)?;
    let rate = Rate::parse(&case.rate).map_err(number)?;
    let fee = Money::parse(&case.fee).map_err(number)?;
    Quote::compute(side, amount, rate, fee).map_err(|e| match e {
        QuoteError::ZeroAmount => "INVALID_NUMBER",
        QuoteError::FeeExceedsPrincipal => "FEE_EXCEEDS_PRINCIPAL",
    })
}

#[test]
fn the_fixture_is_the_one_that_was_generated() {
    let f = fixture();
    assert_eq!(f.cases.len(), 316, "cases");
    assert_eq!(
        f.cases.iter().filter(|c| c.error.is_none()).count(),
        290,
        "accepted"
    );
    // Where v2's float arithmetic and exact decimals disagree, the design keeps exact.
    assert_eq!(
        f.cases
            .iter()
            .filter(|c| c.v2_differs_from_exact == Some(true))
            .count(),
        0
    );
    for c in f.cases.iter().filter(|c| c.error.is_none()) {
        let (v2, exact) = (c.v2.as_ref().unwrap(), c.exact.as_ref().unwrap());
        assert_eq!(
            (&v2.principal, &v2.received, &v2.paid),
            (&exact.principal, &exact.received, &exact.paid)
        );
    }
}

#[test]
fn every_case_v2_accepted_gets_the_same_three_figures() {
    let mut wrong = Vec::new();
    for c in fixture().cases.iter().filter(|c| c.error.is_none()) {
        let want = c.exact.as_ref().unwrap();
        let label = format!("{} {} @ {} fee {}", c.r#type, c.amount, c.rate, c.fee);
        match outcome(c) {
            Ok(q) => {
                let got = (
                    q.principal.to_string(),
                    q.received.to_string(),
                    q.paid.to_string(),
                );
                if got
                    != (
                        want.principal.clone(),
                        want.received.clone(),
                        want.paid.clone(),
                    )
                {
                    wrong.push(format!(
                        "{label}: got {got:?}, want ({}, {}, {})",
                        want.principal, want.received, want.paid
                    ));
                }
            }
            Err(e) => wrong.push(format!("{label}: refused with {e}")),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_case_v2_refused_is_refused_for_the_same_reason() {
    let mut wrong = Vec::new();
    for c in fixture().cases.iter().filter(|c| c.error.is_some()) {
        let want = c.error.as_deref().unwrap();
        let label = format!("{} {:?} @ {:?} fee {:?}", c.r#type, c.amount, c.rate, c.fee);
        match outcome(c) {
            Err(e) if e == want => {}
            Err(e) => wrong.push(format!("{label}: refused with {e}, want {want}")),
            Ok(_) => wrong.push(format!("{label}: accepted, want {want}")),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn half_a_satang_rounds_up() {
    // 0.07 × 0.0715 = 0.005005 → 0.01; 1 × 0.005 = 0.005 → 0.01; 1 × 0.0049 → 0.00.
    let q = |amount: &str, rate: &str| {
        Quote::compute(
            Side::Sell,
            Money::parse(amount).unwrap(),
            Rate::parse(rate).unwrap(),
            Money::ZERO,
        )
        .unwrap()
    };
    assert_eq!(q("0.07", "0.0715").principal.to_string(), "0.01");
    assert_eq!(q("1", "0.005").principal.to_string(), "0.01");
    assert_eq!(q("1", "0.0049").principal.to_string(), "0.00");
    assert_eq!(q("3", "0.0050").principal.to_string(), "0.02"); // 0.015 → 0.02
}

#[test]
fn buying_pays_principal_less_fee_and_selling_takes_principal_plus_fee() {
    let (amount, rate, fee) = (
        Money::parse("100").unwrap(),
        Rate::parse("32.1").unwrap(),
        Money::parse("10.50").unwrap(),
    );
    let buy = Quote::compute(Side::Buy, amount, rate, fee).unwrap();
    assert_eq!(
        (
            buy.principal.to_string(),
            buy.received.to_string(),
            buy.paid.to_string()
        ),
        ("3210.00".into(), "0.00".into(), "3199.50".into())
    );
    let sell = Quote::compute(Side::Sell, amount, rate, fee).unwrap();
    assert_eq!(
        (
            sell.principal.to_string(),
            sell.received.to_string(),
            sell.paid.to_string()
        ),
        ("3210.00".into(), "3220.50".into(), "0.00".into())
    );
}

#[test]
fn a_fee_equal_to_the_purchase_is_allowed_and_one_satang_more_is_not() {
    let (amount, rate) = (Money::parse("100").unwrap(), Rate::parse("32.1").unwrap());
    let all = Quote::compute(Side::Buy, amount, rate, Money::parse("3210").unwrap()).unwrap();
    assert_eq!(all.paid, Money::ZERO);
    assert_eq!(
        Quote::compute(Side::Buy, amount, rate, Money::parse("3210.01").unwrap()),
        Err(QuoteError::FeeExceedsPrincipal)
    );
    // Selling has no such rule.
    assert!(Quote::compute(Side::Sell, amount, rate, Money::parse("99999").unwrap()).is_ok());
}

#[test]
fn a_zero_amount_is_refused_and_the_largest_inputs_still_compute() {
    let rate = Rate::parse("32.1").unwrap();
    assert_eq!(
        Quote::compute(Side::Sell, Money::ZERO, rate, Money::ZERO),
        Err(QuoteError::ZeroAmount)
    );
    // The largest amount at the largest rate with the largest fee: 1e16 + 1e8 baht.
    let max = Money::parse("100000000").unwrap();
    let q = Quote::compute(Side::Sell, max, Rate::parse("100000000").unwrap(), max).unwrap();
    assert_eq!(q.principal.to_string(), "10000000000000000.00");
    assert_eq!(q.received.to_string(), "10000000100000000.00");
    assert_eq!(q.received.minor(), 1_000_000_010_000_000_000);
}
