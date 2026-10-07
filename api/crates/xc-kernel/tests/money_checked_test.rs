//! Provided with ticket #37 (T-37). Written before the implementation: do not edit,
//! delete or weaken.
//!
//! Money arithmetic is checked in the core (`docs/design/xchange-v3.md` §5.1): an
//! amount past what the type holds is a refusal, never a wrapped or cut number.

use xc_kernel::money::{Money, MoneyDelta, NumberError};

/// The largest number of hundredths `Decimal` holds at scale 2 (2^96 − 1).
const DECIMAL_MAX_MINOR: i128 = (1_i128 << 96) - 1;
/// An amount of hundredths that does not fit in 64 bits but fits `Decimal`.
const ABOVE_I64: i128 = 1_i128 << 70;

fn money(text: &str) -> Money {
    Money::parse(text).expect("a plain amount")
}

#[test]
fn checked_plus_refuses_to_pass_the_top_of_i128() {
    assert_eq!(
        MoneyDelta::from_minor(i128::MAX).checked_plus(money("0.01")),
        Err(NumberError::Invalid)
    );
}

#[test]
fn checked_minus_refuses_to_pass_the_bottom_of_i128() {
    assert_eq!(
        MoneyDelta::from_minor(i128::MIN).checked_minus(money("0.01")),
        Err(NumberError::Invalid)
    );
}

#[test]
fn checked_add_refuses_to_leave_i128_either_way() {
    assert_eq!(
        MoneyDelta::from_minor(i128::MAX).checked_add(MoneyDelta::from_minor(1)),
        Err(NumberError::Invalid)
    );
    assert_eq!(
        MoneyDelta::from_minor(i128::MIN).checked_add(MoneyDelta::from_minor(-1)),
        Err(NumberError::Invalid)
    );
}

#[test]
fn the_checked_forms_agree_with_the_plain_ones_whenever_nothing_overflows() {
    let d = MoneyDelta::from_minor(-12_345);
    let m = money("100.50");
    assert_eq!(d.checked_plus(m), Ok(d.plus(m)));
    assert_eq!(d.checked_minus(m), Ok(d.minus(m)));
    assert_eq!(d.checked_add(d), Ok(d.add(d)));
    // right at the edge, still inside
    assert_eq!(
        MoneyDelta::from_minor(i128::MAX - 1).checked_plus(money("0.01")),
        Ok(MoneyDelta::from_minor(i128::MAX))
    );
    assert_eq!(
        MoneyDelta::from_minor(i128::MIN + 1).checked_minus(money("0.01")),
        Ok(MoneyDelta::from_minor(i128::MIN))
    );
    assert_eq!(
        MoneyDelta::from_minor(i128::MAX).checked_add(MoneyDelta::from_minor(-1)),
        Ok(MoneyDelta::from_minor(i128::MAX - 1))
    );
}

/// The gap of the old "without a sixty-four bit limit" test: its value fit `i64`,
/// so `minor_i128` cut to 64 bits survived it. This amount does not fit.
#[test]
fn an_amount_above_sixty_four_bits_survives_try_from_minor_i128_and_minor_i128() {
    let m = Money::try_from_minor_i128(ABOVE_I64).expect("inside the Decimal range");
    assert_eq!(m.minor_i128(), ABOVE_I64);
    assert_eq!(MoneyDelta::from(m).minor(), ABOVE_I64);
}

#[test]
fn try_from_minor_i128_holds_the_decimal_range_and_nothing_past_it() {
    let top = Money::try_from_minor_i128(DECIMAL_MAX_MINOR).expect("the top of the range");
    assert_eq!(top.minor_i128(), DECIMAL_MAX_MINOR);
    assert_eq!(
        Money::try_from_minor_i128(DECIMAL_MAX_MINOR + 1),
        Err(NumberError::Invalid)
    );
    assert_eq!(Money::try_from_minor_i128(-1), Err(NumberError::Invalid));
    assert_eq!(Money::try_from_minor_i128(0), Ok(Money::ZERO));
}

#[test]
fn to_money_refuses_a_delta_past_the_decimal_range() {
    assert_eq!(
        MoneyDelta::from_minor(DECIMAL_MAX_MINOR + 1).to_money(),
        Err(NumberError::Invalid)
    );
    assert_eq!(
        MoneyDelta::from_minor(DECIMAL_MAX_MINOR)
            .to_money()
            .map(Money::minor_i128),
        Ok(DECIMAL_MAX_MINOR)
    );
}
