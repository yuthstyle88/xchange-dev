//! `MoneyDelta`: a signed amount in hundredths — a stock change, a count difference.
//!
//! Provided with ticket T2c. These tests are the specification: make them pass, do not edit them.

use xc_kernel::money::{Money, MoneyDelta, NumberError, Quote, Rate, Side};

fn money(text: &str) -> Money {
    Money::parse(text).expect("valid amount")
}

#[test]
fn zero_is_neither_negative_nor_anything_but_zero() {
    assert_eq!(MoneyDelta::ZERO.minor(), 0);
    assert!(MoneyDelta::ZERO.is_zero());
    assert!(!MoneyDelta::ZERO.is_negative());
}

#[test]
fn hundredths_go_in_and_come_out_unchanged_in_both_signs() {
    for minor in [
        0_i128,
        1,
        -1,
        9_999,
        -9_999,
        320_999_999_968,
        -320_999_999_968,
    ] {
        assert_eq!(MoneyDelta::from_minor(minor).minor(), minor);
    }
}

#[test]
fn a_delta_holds_more_than_sixty_four_bits() {
    // A shift's THB balance is a sum of principals, each up to 100,000,000 × 100,000,000.
    let huge: i128 = 100_000_000_000_000_000_000_000;
    assert_eq!(MoneyDelta::from_minor(huge).minor(), huge);
    assert_eq!(MoneyDelta::from_minor(-huge).minor(), -huge);
}

#[test]
fn money_is_added_and_taken_away_exactly() {
    let delta = MoneyDelta::ZERO.plus(money("100.50")).minus(money("0.51"));
    assert_eq!(delta.minor(), 9_999);
    assert!(!delta.is_negative());

    let below = MoneyDelta::ZERO.plus(money("0.10")).minus(money("0.30"));
    assert_eq!(below.minor(), -20);
    assert!(below.is_negative());
    assert!(!below.is_zero());
}

#[test]
fn point_one_plus_point_two_is_point_three() {
    // The sum a float gets wrong.
    let delta = MoneyDelta::ZERO.plus(money("0.1")).plus(money("0.2"));
    assert_eq!(delta.minor(), 30);
}

#[test]
fn two_deltas_add_with_their_signs() {
    let a = MoneyDelta::from_minor(-500);
    let b = MoneyDelta::from_minor(125);
    assert_eq!(a.add(b).minor(), -375);
    assert_eq!(b.add(a).minor(), -375);
    assert_eq!(a.add(MoneyDelta::ZERO), a);
}

#[test]
fn money_converts_to_a_delta_of_the_same_hundredths() {
    assert_eq!(MoneyDelta::from(money("3210")).minor(), 321_000);
    assert_eq!(MoneyDelta::from(Money::ZERO), MoneyDelta::ZERO);
}

#[test]
fn a_delta_that_is_not_negative_converts_back_to_money() {
    assert_eq!(
        MoneyDelta::from_minor(321_000).to_money(),
        Ok(money("3210"))
    );
    assert_eq!(MoneyDelta::ZERO.to_money(), Ok(Money::ZERO));
}

#[test]
fn a_negative_delta_is_not_money() {
    assert_eq!(
        MoneyDelta::from_minor(-1).to_money(),
        Err(NumberError::Invalid)
    );
}

#[test]
fn it_prints_two_decimals_with_a_leading_minus_and_no_grouping() {
    assert_eq!(MoneyDelta::ZERO.to_string(), "0.00");
    assert_eq!(MoneyDelta::from_minor(5).to_string(), "0.05");
    assert_eq!(MoneyDelta::from_minor(-5).to_string(), "-0.05");
    assert_eq!(MoneyDelta::from_minor(-1_234).to_string(), "-12.34");
    assert_eq!(
        MoneyDelta::from_minor(320_999_999_968).to_string(),
        "3209999999.68"
    );
}

#[test]
fn deltas_order_by_value() {
    assert!(MoneyDelta::from_minor(-1) < MoneyDelta::ZERO);
    assert!(MoneyDelta::ZERO < MoneyDelta::from_minor(1));
}

#[test]
fn money_gives_its_hundredths_without_a_sixty_four_bit_limit() {
    // v2 stores this principal (99,999,999.99 × 32.1); see the quote fixture.
    let quote = Quote::compute(
        Side::Sell,
        money("99999999.99"),
        Rate::parse("32.1").unwrap(),
        Money::ZERO,
    )
    .expect("a valid sale");
    assert_eq!(quote.principal.minor_i128(), 320_999_999_968);
    assert_eq!(MoneyDelta::from(quote.principal).minor(), 320_999_999_968);
    assert_eq!(money("0.07").minor_i128(), 7);
}
