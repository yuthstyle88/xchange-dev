//! `Quote::per_unit` refuses an amount whose baht overflows `Decimal`, instead of panicking
//! (tracking #2063, K-a caller migration; design `docs/design/blueprint/bounded-inputs.md` §6a).
//!
//! Written before the implementation by the ticket author (claude/d7aa64). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Today `per_unit` prices with `ExchangeArithmetic::thb`, whose `Decimal` `*` and `/` panic on
//! overflow ("Multiplication overflowed" / "Division overflowed", rust_decimal 1.43
//! `arithmetic_impls.rs`). After the fix it calls `try_thb` and answers
//! `Err(QuoteError::ArithmeticOverflow)`, displayed as "the amount is too large to price".
//!
//! The new variant is pinned by its `Debug` name and its `Display` text, not by naming it, so this
//! file compiles before the variant exists and fails by the panic, not by a compile error.
//! `per_unit` has no production caller (§6a), so these tests are kernel-level.
//!
//! The implementer also adds one arm for the new variant to the exhaustive match in
//! `tests/quote_test.rs` (`outcome`, `:72-75`); that is the ticket's one listed edit to a provided
//! test, and it is not made here.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{ForeignAmount, Money, Quote, QuoteError, Rate, Side, UnitSize};

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn foreign(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(Decimal::from_str(text).expect("a decimal"))
}

fn assert_too_large(result: Result<Quote, QuoteError>, label: &str) {
    let err = result.expect_err(label);
    assert_eq!(format!("{err:?}"), "ArithmeticOverflow", "{label}");
    assert_eq!(
        err.to_string(),
        "the amount is too large to price",
        "{label}"
    );
}

// ── 1. an overflowing amount is refused on both sides ───────────────────────

#[test]
fn a_product_past_decimal_is_refused_on_both_sides() {
    // Decimal::MAX × 100,000,000 does not fit.
    for side in [Side::Buy, Side::Sell] {
        assert_too_large(
            Quote::per_unit(
                side,
                ForeignAmount::from_decimal(Decimal::MAX),
                rate("100000000"),
                UnitSize::ONE,
                Money::ZERO,
            ),
            &format!("{side:?}: Decimal::MAX × 100,000,000"),
        );
    }
}

#[test]
fn a_quotient_past_decimal_is_refused_on_both_sides() {
    // Decimal::MAX × 1 fits; ÷ 0.0001 does not.
    for side in [Side::Buy, Side::Sell] {
        assert_too_large(
            Quote::per_unit(
                side,
                ForeignAmount::from_decimal(Decimal::MAX),
                rate("1"),
                unit("0.0001"),
                Money::ZERO,
            ),
            &format!("{side:?}: Decimal::MAX ÷ 0.0001"),
        );
    }
}

#[test]
fn the_overflow_is_refused_whatever_the_fee() {
    // The refusal comes from pricing, before the fee is compared or added.
    assert_too_large(
        Quote::per_unit(
            Side::Buy,
            ForeignAmount::from_decimal(Decimal::MAX),
            rate("100000000"),
            UnitSize::ONE,
            money("100"),
        ),
        "Buy with a fee",
    );
    assert_too_large(
        Quote::per_unit(
            Side::Sell,
            ForeignAmount::from_decimal(Decimal::MAX),
            rate("100000000"),
            UnitSize::ONE,
            money("100"),
        ),
        "Sell with a fee",
    );
}

// ── 2. what per_unit answered before is unchanged (keep-pin) ────────────────

#[test]
fn a_zero_amount_is_still_refused_as_zero() {
    assert_eq!(
        Quote::per_unit(
            Side::Sell,
            foreign("0"),
            rate("33.50"),
            UnitSize::ONE,
            Money::ZERO
        ),
        Err(QuoteError::ZeroAmount)
    );
}

#[test]
fn ordinary_quotes_are_unchanged() {
    // 100 USD at 33.50 per 1, the shop sells, fee 20: principal 3,350.00, received 3,370.00.
    let sell = Quote::per_unit(
        Side::Sell,
        foreign("100"),
        rate("33.50"),
        UnitSize::ONE,
        money("20"),
    )
    .expect("an ordinary sell");
    assert_eq!(
        (
            sell.principal.to_string(),
            sell.received.to_string(),
            sell.paid.to_string()
        ),
        ("3350.00".into(), "3370.00".into(), "0.00".into())
    );
    // 100 USD at 33.50 per 1, the shop buys, fee 50: paid 3,300.00.
    let buy = Quote::per_unit(
        Side::Buy,
        foreign("100"),
        rate("33.50"),
        UnitSize::ONE,
        money("50"),
    )
    .expect("an ordinary buy");
    assert_eq!(
        (
            buy.principal.to_string(),
            buy.received.to_string(),
            buy.paid.to_string()
        ),
        ("3350.00".into(), "0.00".into(), "3300.00".into())
    );
    // 1,000 JPY at 0.2345 per 100 = 2.345 → 2.35 (half up, once).
    let jpy = Quote::per_unit(
        Side::Sell,
        foreign("1000"),
        rate("0.2345"),
        unit("100"),
        Money::ZERO,
    )
    .expect("an ordinary JPY quote");
    assert_eq!(jpy.principal, money("2.35"));
}

#[test]
fn a_fee_above_the_principal_is_still_refused_as_before() {
    assert_eq!(
        Quote::per_unit(
            Side::Buy,
            foreign("1"),
            rate("33.50"),
            UnitSize::ONE,
            money("40")
        ),
        Err(QuoteError::FeeExceedsPrincipal)
    );
}
