//! pricing-formula wave 1 (docs/design/blueprint/pricing-formula.md §2; B2, B10, B12): `FormulaText::parse` and
//! `FormulaExpr::eval`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Pure: no database. `eval` is tested on hand-built expressions so it does not depend on `parse`.

use rust_decimal::Decimal;
use std::str::FromStr;
use xc_rate::domain::formula::error::FormulaError;
use xc_rate::domain::formula::expr::{FormulaExpr, Op};
use xc_rate::domain::formula::input::{FormulaInput, FormulaInputs};
use xc_rate::domain::formula::price_formula::{DEFAULT_BUY, DEFAULT_SELL};
use xc_rate::domain::formula::text::FormulaText;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn usd() -> FormulaInputs {
    FormulaInputs {
        ref_buy: Some(d("33.6482")),
        ref_sell: Some(d("33.9671")),
        buy_pct: d("0.3"),
        sell_pct: d("-0.3"),
        tier_buy_pct: d("0.1"),
        tier_sell_pct: d("-0.1"),
    }
}

/// `text` parsed and evaluated on `inputs`.
fn value(text: &str, inputs: &FormulaInputs) -> Result<Decimal, FormulaError> {
    FormulaText::parse(text)?.expr().eval(inputs)
}

fn num(s: &str) -> Box<FormulaExpr> {
    Box::new(FormulaExpr::Num(d(s)))
}

fn input(i: FormulaInput) -> Box<FormulaExpr> {
    Box::new(FormulaExpr::Input(i))
}

// ── parse ───────────────────────────────────────────────────────────────────

#[test]
fn b2_the_default_texts_parse() {
    for text in [DEFAULT_BUY, DEFAULT_SELL] {
        let f = FormulaText::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(f.as_str(), text);
    }
}

#[test]
fn the_text_is_kept_trimmed() {
    assert_eq!(
        FormulaText::parse("  ref_buy  ").expect("parses").as_str(),
        "ref_buy"
    );
}

#[test]
fn every_input_name_parses_to_its_input() {
    for i in FormulaInput::ALL {
        let f = FormulaText::parse(i.name()).unwrap_or_else(|e| panic!("{}: {e}", i.name()));
        assert_eq!(f.expr(), &FormulaExpr::Input(i), "{}", i.name());
    }
}

#[test]
fn parse_builds_the_expected_tree() {
    let f = FormulaText::parse("min(ref_buy * 0.997, ref_mid - 0.03)").expect("parses");
    assert_eq!(
        f.expr(),
        &FormulaExpr::Min(
            Box::new(FormulaExpr::Bin(Op::Mul, input(FormulaInput::RefBuy), num("0.997"))),
            Box::new(FormulaExpr::Bin(Op::Sub, input(FormulaInput::RefMid), num("0.03"))),
        )
    );
    assert_eq!(
        FormulaText::parse("-ref_buy").expect("parses").expr(),
        &FormulaExpr::Neg(input(FormulaInput::RefBuy))
    );
}

#[test]
fn precedence_and_left_associativity() {
    let none = FormulaInputs::default();
    for (text, want) in [
        ("1 + 2 * 3", "7"),
        ("(1 + 2) * 3", "9"),
        ("10 - 4 - 3", "3"),
        ("8 / 4 / 2", "1"),
        ("-2 * 3", "-6"),
        ("2 - -3", "5"),
        ("max(1, 2) + min(5, 4)", "6"),
        ("max(min(1, 2), 0.5)", "1"),
        ("0.12345678 * 100", "12.345678"),
    ] {
        assert_eq!(value(text, &none), Ok(d(want)), "{text}");
    }
}

#[test]
fn b12_an_unknown_name_is_unknown_input() {
    assert_eq!(
        FormulaText::parse("foo + 1"),
        Err(FormulaError::UnknownInput("foo".into()))
    );
    assert_eq!(
        FormulaText::parse("ref_buy * pow(2, 3)"),
        Err(FormulaError::UnknownInput("pow".into()))
    );
}

#[test]
fn b12_malformed_text_is_a_syntax_error() {
    for text in [
        "ref_buy +",
        "",
        "   ",
        "(ref_buy",
        "ref_buy)",
        "ref_buy ref_sell",
        "min(ref_buy)",
        "min(ref_buy, 1, 2)",
        "1e5",
        ".5",
        "5.",
        "0.123456789",
        "ref_buy % 2",
        "ref_buy ** 2",
    ] {
        assert!(
            matches!(FormulaText::parse(text), Err(FormulaError::Syntax { .. })),
            "{text:?}: {:?}",
            FormulaText::parse(text)
        );
    }
}

#[test]
fn b12_501_characters_is_too_long_and_500_is_not() {
    // "1" followed by "+1" pairs: 1 + 2n characters
    let at_500 = format!("{}{}", "1 ", "+1".repeat(249));
    assert_eq!(at_500.chars().count(), 500);
    assert!(FormulaText::parse(&at_500).is_ok(), "500 characters parse");
    let at_501 = format!("{at_500} ");
    let at_501 = format!("{}1", &at_501[..500]);
    assert_eq!(at_501.chars().count(), 501);
    assert_eq!(FormulaText::parse(&at_501), Err(FormulaError::TooLong));
}

#[test]
fn b12_17_nested_parentheses_is_too_deep_and_16_is_not() {
    let nest = |n: usize| format!("{}ref_buy{}", "(".repeat(n), ")".repeat(n));
    assert!(FormulaText::parse(&nest(16)).is_ok(), "16 levels parse");
    assert_eq!(FormulaText::parse(&nest(17)), Err(FormulaError::TooDeep));
    // a function call's parentheses count too
    let calls = format!("{}1, 1{}", "min(".repeat(17), ")".repeat(17));
    assert_eq!(FormulaText::parse(&calls), Err(FormulaError::TooDeep));
}

// ── eval ────────────────────────────────────────────────────────────────────

#[test]
fn eval_reads_every_input() {
    let i = usd();
    for (input, want) in [
        (FormulaInput::RefBuy, "33.6482"),
        (FormulaInput::RefSell, "33.9671"),
        (FormulaInput::RefMid, "33.80765"),
        (FormulaInput::BuyPct, "0.3"),
        (FormulaInput::SellPct, "-0.3"),
        (FormulaInput::TierBuyPct, "0.1"),
        (FormulaInput::TierSellPct, "-0.1"),
    ] {
        assert_eq!(
            FormulaExpr::Input(input).eval(&i),
            Ok(d(want)),
            "{}",
            input.name()
        );
    }
}

#[test]
fn eval_computes_exactly_without_rounding() {
    // ref_mid - 0.05 on the 10-28 USD line (B3, before rounding)
    let e = FormulaExpr::Bin(Op::Sub, input(FormulaInput::RefMid), num("0.05"));
    assert_eq!(e.eval(&usd()), Ok(d("33.75765")));
    // 1 / 3 keeps rust_decimal's full precision: no rounding here
    let third = FormulaExpr::Bin(Op::Div, num("1"), num("3"));
    assert_eq!(third.eval(&usd()), Ok(Decimal::ONE / Decimal::from(3)));
    // a negative result is not refused by eval (PriceFormula refuses it)
    let neg = FormulaExpr::Neg(input(FormulaInput::RefBuy));
    assert_eq!(neg.eval(&usd()), Ok(d("-33.6482")));
    let max = FormulaExpr::Max(num("1"), num("2.5"));
    assert_eq!(max.eval(&usd()), Ok(d("2.5")));
    let min = FormulaExpr::Min(num("1"), num("2.5"));
    assert_eq!(min.eval(&usd()), Ok(d("1")));
}

#[test]
fn b10_division_by_zero() {
    // ref_buy / (buy_pct - buy_pct)
    let e = FormulaExpr::Bin(
        Op::Div,
        input(FormulaInput::RefBuy),
        Box::new(FormulaExpr::Bin(
            Op::Sub,
            input(FormulaInput::BuyPct),
            input(FormulaInput::BuyPct),
        )),
    );
    assert_eq!(e.eval(&usd()), Err(FormulaError::DivideByZero));
    assert_eq!(
        value("ref_buy / (buy_pct - buy_pct)", &usd()),
        Err(FormulaError::DivideByZero)
    );
}

#[test]
fn a_missing_reference_side_is_missing_input() {
    let buy_only = FormulaInputs {
        ref_sell: None,
        ..usd()
    };
    assert_eq!(
        FormulaExpr::Input(FormulaInput::RefSell).eval(&buy_only),
        Err(FormulaError::MissingInput(FormulaInput::RefSell))
    );
    assert_eq!(
        FormulaExpr::Input(FormulaInput::RefMid).eval(&buy_only),
        Err(FormulaError::MissingInput(FormulaInput::RefMid))
    );
    // a formula that does not name the missing side still evaluates
    assert_eq!(
        FormulaExpr::Input(FormulaInput::RefBuy).eval(&buy_only),
        Ok(d("33.6482"))
    );
}

#[test]
fn an_overflow_is_overflow_not_a_panic() {
    let big = FormulaExpr::Bin(Op::Mul, Box::new(FormulaExpr::Num(Decimal::MAX)), num("10"));
    assert_eq!(big.eval(&usd()), Err(FormulaError::Overflow));
    let sum = FormulaExpr::Bin(Op::Add, Box::new(FormulaExpr::Num(Decimal::MAX)), num("1"));
    assert_eq!(sum.eval(&usd()), Err(FormulaError::Overflow));
}
