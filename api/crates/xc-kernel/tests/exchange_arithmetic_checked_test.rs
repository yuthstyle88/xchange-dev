//! Checked exchange arithmetic: `ExchangeArithmetic::try_thb` and `try_foreign_for_thb`.
//! Written before the implementation by the ticket author (claude/d7aa64). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Today `thb` and `foreign_for_thb` use `Decimal`'s `*` and `/`, which panic on overflow
//! ("Multiplication overflowed" / "Division overflowed", rust_decimal 1.43
//! `arithmetic_impls.rs`). The checked forms return `Err(ArithmeticOverflow)` instead and
//! otherwise give exactly what the unchecked forms give: HALF_UP to 2 places once for THB,
//! truncation (toward zero) to the currency's minor unit for foreign.
//!
//! The error type is pinned only by name (`xc_kernel::money::ArithmeticOverflow`), not by
//! shape: these tests never construct it.

use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};
use xc_kernel::money::{
    ArithmeticOverflow, CurrencyCode, CurrencyUnits, ExchangeArithmetic, ForeignAmount, MinorUnit,
    Money, Rate, UnitSize,
};

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn units(code: &str, unit: &str, minor: u8) -> CurrencyUnits {
    CurrencyUnits::new(
        CurrencyCode::parse(code).expect("a known code"),
        UnitSize::parse(unit).expect("a unit size"),
        MinorUnit::new(minor).expect("a minor unit"),
    )
}

fn foreign(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(text))
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

/// The largest amount `Money` holds: (2^96 − 1) hundredths.
fn money_at_decimal_max() -> Money {
    Money::try_from_minor_i128((1_i128 << 96) - 1).expect("fits Decimal at scale 2")
}

/// `try_thb` agrees with the HALF_UP formula in value and in printed form (two places).
fn assert_thb_same(amount: ForeignAmount, r: Rate, u: UnitSize) {
    let old = format!(
        "{:.2}",
        (amount.value() * r.value() / u.value())
            .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
    );
    let new: Money = ExchangeArithmetic::try_thb(amount, r, u)
        .unwrap_or_else(|e| panic!("{amount:?} × {r} ÷ {u}: unexpected {e:?}"));
    assert_eq!(new.to_string(), old, "{amount:?} × {r} ÷ {u}");
}

/// `try_foreign_for_thb` agrees with the truncated formula in value and in printed form.
fn assert_foreign_same(thb: Money, r: Rate, u: &CurrencyUnits) {
    let minor = u.minor_unit().get() as u32;
    let old = ForeignAmount::from_decimal(
        (d(&thb.to_string()) * u.unit_size().value() / r.value())
            .round_dp_with_strategy(minor, RoundingStrategy::ToZero),
    );
    let new: ForeignAmount = ExchangeArithmetic::try_foreign_for_thb(thb, r, u)
        .unwrap_or_else(|e| panic!("{thb} ÷ {r} × {u:?}: unexpected {e:?}"));
    assert_eq!(new, old, "{thb} ÷ {r} × {u:?}");
    assert_eq!(new.to_string(), old.to_string(), "{thb} ÷ {r} × {u:?}");
}

// ── 1. try_thb on ordinary values = thb ─────────────────────────────────────

#[test]
fn try_thb_gives_what_thb_gives_for_ordinary_values() {
    // 100 USD at 33.50 per 1 = 3350.00
    assert_eq!(
        ExchangeArithmetic::try_thb(foreign("100"), rate("33.50"), UnitSize::ONE)
            .expect("no overflow")
            .to_string(),
        "3350.00"
    );
    assert_thb_same(foreign("100"), rate("33.50"), UnitSize::ONE);
    // 1,000 JPY at 0.2345 per 100 = 2.345 → 2.35 (HALF_UP; banker's would give 2.34)
    assert_eq!(
        ExchangeArithmetic::try_thb(foreign("1000"), rate("0.2345"), unit("100"))
            .expect("no overflow"),
        money("2.35")
    );
    assert_thb_same(foreign("1000"), rate("0.2345"), unit("100"));
}

#[test]
fn try_thb_rounds_half_up_once_at_the_end() {
    // 3 × 0.335 = 1.005 → 1.01, never 1.00
    assert_eq!(
        ExchangeArithmetic::try_thb(foreign("3"), rate("0.335"), UnitSize::ONE)
            .expect("no overflow"),
        money("1.01")
    );
    // 1 × 0.004 → 0.00; 1 × 0.005 → 0.01
    assert_eq!(
        ExchangeArithmetic::try_thb(foreign("1"), rate("0.004"), UnitSize::ONE)
            .expect("no overflow"),
        money("0")
    );
    assert_eq!(
        ExchangeArithmetic::try_thb(foreign("1"), rate("0.005"), UnitSize::ONE)
            .expect("no overflow"),
        money("0.01")
    );
    // once, at the end: 7 × 0.715 ÷ 100 = 0.05005 → 0.05
    assert_eq!(
        ExchangeArithmetic::try_thb(foreign("7"), rate("0.715"), unit("100")).expect("no overflow"),
        money("0.05")
    );
}

// ── 2. try_thb refuses an overflow instead of panicking ─────────────────────

#[test]
fn try_thb_refuses_a_product_past_decimal() {
    // Decimal::MAX × 100,000,000 does not fit; thb panics "Multiplication overflowed".
    let r = ExchangeArithmetic::try_thb(
        ForeignAmount::from_decimal(Decimal::MAX),
        rate("100000000"),
        UnitSize::ONE,
    );
    let _: ArithmeticOverflow = r.expect_err("the product overflows Decimal");
}

#[test]
fn try_thb_refuses_a_quotient_past_decimal() {
    // The product fits (× 1), but ÷ 0.0001 does not; thb panics "Division overflowed".
    let r = ExchangeArithmetic::try_thb(
        ForeignAmount::from_decimal(Decimal::MAX),
        rate("1"),
        unit("0.0001"),
    );
    let _: ArithmeticOverflow = r.expect_err("the quotient overflows Decimal");
}

// ── 3. try_foreign_for_thb on ordinary values = foreign_for_thb ─────────────

#[test]
fn try_foreign_for_thb_gives_what_foreign_for_thb_gives() {
    let jpy = units("JPY", "100", 0);
    let usd = units("USD", "1", 2);
    let bhd = units("BHD", "1", 3);
    // 100.00 THB at 23.5 per 100 JPY = 425.53… → 425 (minor unit 0, truncated)
    assert_eq!(
        ExchangeArithmetic::try_foreign_for_thb(money("100"), rate("23.5"), &jpy)
            .expect("no overflow"),
        foreign("425")
    );
    // 100.00 THB at 32.1 per USD = 3.11526… → 3.11 (not 3.12: toward zero)
    assert_eq!(
        ExchangeArithmetic::try_foreign_for_thb(money("100"), rate("32.1"), &usd)
            .expect("no overflow"),
        foreign("3.11")
    );
    // 100.00 THB at 85.12345 per BHD = 1.17476… → 1.174
    assert_eq!(
        ExchangeArithmetic::try_foreign_for_thb(money("100"), rate("85.12345"), &bhd)
            .expect("no overflow"),
        foreign("1.174")
    );
    // 10.00 THB at 3 per USD = 3.333… → 3.33
    assert_eq!(
        ExchangeArithmetic::try_foreign_for_thb(money("10"), rate("3"), &usd).expect("no overflow"),
        foreign("3.33")
    );
    assert_foreign_same(money("100"), rate("23.5"), &jpy);
    assert_foreign_same(money("100"), rate("32.1"), &usd);
    assert_foreign_same(money("100"), rate("85.12345"), &bhd);
    assert_foreign_same(money("10"), rate("3"), &usd);
}

// ── 4. try_foreign_for_thb refuses an overflow instead of panicking ─────────

#[test]
fn try_foreign_for_thb_refuses_a_product_past_decimal() {
    // (2^96 − 1) hundredths × unit 1,000,000 does not fit; foreign_for_thb panics.
    let r = ExchangeArithmetic::try_foreign_for_thb(
        money_at_decimal_max(),
        rate("1"),
        &units("JPY", "1000000", 0),
    );
    let _: ArithmeticOverflow = r.expect_err("the product overflows Decimal");
}

#[test]
fn try_foreign_for_thb_refuses_a_quotient_past_decimal() {
    // × 1 fits, ÷ 0.00000001 does not; foreign_for_thb panics "Division overflowed".
    let r = ExchangeArithmetic::try_foreign_for_thb(
        money_at_decimal_max(),
        rate("0.00000001"),
        &units("USD", "1", 2),
    );
    let _: ArithmeticOverflow = r.expect_err("the quotient overflows Decimal");
}

// ── 5. Across representative inputs, checked = unchecked ───────────────────

#[test]
fn try_thb_matches_thb_across_representative_inputs() {
    let amounts = [
        "0.01",
        "1",
        "3",
        "7",
        "1000",
        "12345.67",
        "99999999.99",
        "100000000",
    ];
    let rates = [
        "0.00000001",
        "0.004",
        "0.005",
        "0.335",
        "0.715",
        "23.5",
        "25.12345678",
        "32.1",
        "100000000",
    ];
    let unit_sizes = ["0.0001", "1", "100", "1000"];
    for a in amounts {
        for r in rates {
            for u in unit_sizes {
                assert_thb_same(foreign(a), rate(r), unit(u));
            }
        }
    }
}

#[test]
fn try_foreign_for_thb_matches_foreign_for_thb_across_representative_inputs() {
    let thbs = [
        "0.01",
        "1",
        "10",
        "100",
        "12345.67",
        "99999999.99",
        "100000000",
    ];
    let rates = [
        "0.00000001",
        "0.2195",
        "3",
        "23.5",
        "25.12345678",
        "32.1",
        "85.12345",
        "100000000",
    ];
    let currencies = [
        units("USD", "1", 2),
        units("JPY", "100", 0),
        units("KRW", "1000", 0),
        units("BHD", "1", 3),
    ];
    for t in thbs {
        for r in rates {
            for u in &currencies {
                assert_foreign_same(money(t), rate(r), u);
            }
        }
    }
}
