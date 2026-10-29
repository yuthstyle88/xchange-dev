//! pricing-formula wave 1 (docs/design/blueprint/pricing-formula.md §2; B1 unit half, B2, B3, B11):
//! `PriceFormula::default_formula`, `side` and `price`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Pure: no database. B1's unit half: the default formula gives `RateProposal::of`'s price (main `bbc4d473`) to the
//! 8th place, for both sides, with and without a tier margin, at unit size 1 and 100.

use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, Side, UnitSize};
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_proposal::RateProposal;
use xc_rate::domain::formula::error::FormulaError;
use xc_rate::domain::formula::input::FormulaInputs;
use xc_rate::domain::formula::price_formula::{DEFAULT_BUY, DEFAULT_SELL, PriceFormula};
use xc_rate::domain::formula::text::FormulaText;
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::domain::reference::reference_batch::ReferenceLine;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn rate(s: &str) -> Rate {
    Rate::parse(s).expect("a rate")
}

fn formula(buy: &str, sell: &str) -> PriceFormula {
    PriceFormula::new(
        FormulaText::parse(buy).expect("buy parses"),
        FormulaText::parse(sell).expect("sell parses"),
    )
}

fn inputs(ref_buy: &str, ref_sell: &str, buy_pct: &str, sell_pct: &str) -> FormulaInputs {
    FormulaInputs {
        ref_buy: Some(d(ref_buy)),
        ref_sell: Some(d(ref_sell)),
        buy_pct: d(buy_pct),
        sell_pct: d(sell_pct),
        tier_buy_pct: Decimal::ZERO,
        tier_sell_pct: Decimal::ZERO,
    }
}

#[test]
fn b2_the_default_is_the_default_text() {
    let f = PriceFormula::default_formula();
    assert_eq!(f.buy.as_str(), DEFAULT_BUY);
    assert_eq!(f.sell.as_str(), DEFAULT_SELL);
    assert_eq!(f, formula(DEFAULT_BUY, DEFAULT_SELL));
    assert_eq!(PriceFormula::default(), f);
    assert!(f.is_default());
    assert!(!formula("ref_mid - 0.05", "ref_mid + 0.05").is_default());
}

#[test]
fn b3_fixed_satang_either_side_of_the_middle() {
    let f = formula("ref_mid - 0.05", "ref_mid + 0.05");
    let (buy, sell) = f
        .price(&inputs("33.6482", "33.9671", "0", "0"))
        .expect("prices");
    assert_eq!(buy, rate("33.75765"));
    assert_eq!(sell, rate("33.85765"));
}

#[test]
fn side_rounds_to_8_places_half_away_from_zero() {
    // 1.000000005 → 1.00000001 (half away from zero; banker's would give 1.00000000)
    let f = formula("ref_buy + 0.00000000 + 1 / 200000000", "ref_sell");
    let i = inputs("1", "2", "0", "0");
    assert_eq!(f.side(Side::Buy, &i), Ok(rate("1.00000001")));
    // 2.000000025 → 2.00000003
    let g = formula("ref_buy", "ref_sell + 1 / 40000000");
    assert_eq!(g.side(Side::Sell, &i), Ok(rate("2.00000003")));
}

#[test]
fn b11_a_price_of_zero_or_below_is_not_positive() {
    let i = inputs("33.6482", "33.9671", "0", "0");
    let zero = formula("ref_buy * 0", "ref_sell");
    assert_eq!(zero.side(Side::Buy, &i), Err(FormulaError::NotPositive));
    assert_eq!(zero.price(&i), Err(FormulaError::NotPositive));
    // rounds to zero at the 8th place
    let tiny = formula("ref_buy", "0.00000004 / 10");
    assert_eq!(tiny.side(Side::Sell, &i), Err(FormulaError::NotPositive));
    let negative = formula("ref_buy - 40", "ref_sell");
    assert_eq!(negative.side(Side::Buy, &i), Err(FormulaError::NotPositive));
}

#[test]
fn evaluation_errors_pass_through_and_the_buy_side_wins() {
    let i = inputs("33.6482", "33.9671", "0", "0");
    let f = formula("ref_buy / (buy_pct - buy_pct)", "ref_sell * 0");
    assert_eq!(f.side(Side::Buy, &i), Err(FormulaError::DivideByZero));
    assert_eq!(f.price(&i), Err(FormulaError::DivideByZero));
    let buy_only = FormulaInputs {
        ref_sell: None,
        ..i
    };
    assert_eq!(
        PriceFormula::default_formula().side(Side::Sell, &buy_only),
        Err(xc_rate::domain::formula::error::FormulaError::MissingInput(
            xc_rate::domain::formula::input::FormulaInput::RefSell
        ))
    );
}

#[test]
fn price_does_not_check_buy_against_sell() {
    // +0.7 / -0.7 on the 10-28 USD line inverts (B4's numbers); price answers both, the check is the caller's
    let (buy, sell) = PriceFormula::default_formula()
        .price(&inputs("33.6482", "33.9671", "0.7", "-0.7"))
        .expect("prices");
    assert_eq!(buy, rate("33.8837374"));
    assert_eq!(sell, rate("33.7293303"));
}

// ── B1, unit half: the default formula = today's RateProposal::of ───────────

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).expect("a code")
}

fn units(c: &str, size: &str) -> CurrencyUnits {
    CurrencyUnits::new(
        code(c),
        UnitSize::parse(size).expect("a unit size"),
        MinorUnit::new(2).expect("a minor unit"),
    )
}

fn policy(buy: &str, sell: &str) -> RatePolicy {
    RatePolicy::new(
        String::from("p1"),
        RatePercent::parse(buy).expect("pct"),
        RatePercent::parse(sell).expect("pct"),
        &json!({}),
    )
    .expect("a policy")
}

#[test]
fn b1_the_default_formula_gives_todays_auto_price() {
    let cases = [
        // (currency, unit size of the currency, reference unit size, ref buy, ref sell, buy %, sell %)
        ("USD", "1", "1", "33.6482", "33.9671", "0.3", "-0.3"),
        ("USD", "1", "1", "33.6482", "33.9671", "0", "0"),
        (
            "USD",
            "1",
            "1",
            "33.12345678",
            "33.98765432",
            "-1.2345",
            "2.3456",
        ),
        ("EUR", "1", "1", "36.40125", "37.10333", "0.0001", "-0.0001"),
        ("JPY", "100", "1", "0.215432", "0.221987", "0.3", "-0.3"),
        ("JPY", "100", "100", "21.5432", "22.1987", "0.5", "-0.5"),
    ];
    for (c, size, ref_size, rb, rs, bp, sp) in cases {
        let u = units(c, size);
        let reference = ReferenceLine::new(
            code(c),
            UnitSize::parse(ref_size).expect("ref unit"),
            Some(rate(rb)),
            Some(rate(rs)),
        );
        let line = DraftLine::new(code(c), LineSide::Auto, LineSide::Auto, None);
        let today = RateProposal::of(&line, Some(&reference), Some(&policy(bp, sp)), &u)
            .expect("today's proposal");
        // the inputs at the currency's unit size, as §2 binds them
        let scale = d(size) / d(ref_size);
        let i = FormulaInputs {
            ref_buy: Some(d(rb) * scale),
            ref_sell: Some(d(rs) * scale),
            buy_pct: d(bp),
            sell_pct: d(sp),
            tier_buy_pct: Decimal::ZERO,
            tier_sell_pct: Decimal::ZERO,
        };
        let f = PriceFormula::default_formula();
        assert_eq!(
            Some(f.side(Side::Buy, &i).expect("buy")),
            today.buy,
            "{c} {rb}/{rs} {bp}/{sp}: buy"
        );
        assert_eq!(
            Some(f.side(Side::Sell, &i).expect("sell")),
            today.sell,
            "{c} {rb}/{rs} {bp}/{sp}: sell"
        );
    }
}

#[test]
fn b1_the_default_formula_with_a_tier_margin_adds_it_to_the_percentage() {
    // tier_proposal.rs:63-66: effective = buy_adjust_pct + margin
    let i = FormulaInputs {
        tier_buy_pct: d("0.14"),
        tier_sell_pct: d("-0.14"),
        ..inputs("33.6482", "33.9671", "0.3", "-0.3")
    };
    let f = PriceFormula::default_formula();
    let want_buy = (d("33.6482") * (Decimal::ONE + d("0.44") / Decimal::from(100)))
        .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let want_sell = (d("33.9671") * (Decimal::ONE + d("-0.44") / Decimal::from(100)))
        .round_dp_with_strategy(8, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    assert_eq!(f.side(Side::Buy, &i).expect("buy").value(), want_buy);
    assert_eq!(f.side(Side::Sell, &i).expect("sell").value(), want_sell);
}
