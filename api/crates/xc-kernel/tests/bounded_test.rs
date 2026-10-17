//! K-1 up-front tests (#2054, design `docs/design/blueprint/bounded-inputs.md` §2 B-1, B-7, §3 `:52-85`) — written
//! before the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller
//! claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `Bound::parse` must be exactly as strict as today's `parse_decimal` (`money.rs:102-151`): no trimming; digits and
//! one '.', no leading or trailing '.'; a leading '-' only under `Sign::Signed`, never '+'; extra decimals refused
//! unless they are zeros; out of range refused. Both sides of every bound are pinned (#2054 issuecomment-6201140913).

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{Bound, BoundError, IntBound, Sign};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("test decimal")
}

/// `numeric(20,2)`, the cash-limit / money column shape.
const N20_2: Bound = Bound::numeric(20, 2, Sign::NonNegative);

fn ok(b: &Bound, text: &str, want: &str) {
    match b.parse(text) {
        Ok(v) => assert_eq!(v, d(want), "{text:?} parses to {want}"),
        Err(e) => panic!("{text:?} must be accepted as {want}, got {e:?}"),
    }
}

fn err(b: &Bound, text: &str, want: BoundError) {
    assert_eq!(b.parse(text), Err(want), "{text:?}");
}

// ---------------------------------------------------------------- syntax (design :82)

#[test]
fn syntax_that_is_not_a_plain_decimal_is_refused() {
    for b in [
        Bound::numeric(20, 2, Sign::Positive),
        Bound::numeric(20, 2, Sign::NonNegative),
        Bound::numeric(9, 4, Sign::Signed),
    ] {
        for text in [
            "", " 1", "1 ", "+1", ".5", "5.", "1e5", "1.2.3", "1,000", "abc", "-", "--1", "0x10",
        ] {
            err(&b, text, BoundError::Syntax);
        }
    }
}

#[test]
fn plain_decimals_are_accepted() {
    ok(&N20_2, "0", "0");
    ok(&N20_2, "7", "7");
    ok(&N20_2, "100.5", "100.5");
    ok(&N20_2, "0.07", "0.07");
    ok(&N20_2, "00012.30", "12.3");
}

// ---------------------------------------------------------------- scale (design :84)

#[test]
fn extra_decimals_that_are_zeros_are_accepted() {
    ok(&N20_2, "1.230", "1.23");
    ok(&N20_2, "1.2300000", "1.23");
}

#[test]
fn a_non_zero_extra_decimal_is_too_many_decimals_with_the_scale() {
    err(&N20_2, "1.234", BoundError::TooManyDecimals { max: 2 });
    err(&N20_2, "0.001", BoundError::TooManyDecimals { max: 2 });
    err(
        &Bound::numeric(20, 4, Sign::Positive),
        "0.00001",
        BoundError::TooManyDecimals { max: 4 },
    );
    err(
        &Bound::numeric(20, 0, Sign::Positive),
        "1.5",
        BoundError::TooManyDecimals { max: 0 },
    );
}

// ---------------------------------------------------------------- sign (design :83)

#[test]
fn positive_refuses_zero_and_negatives_as_sign() {
    let b = Bound::numeric(20, 4, Sign::Positive);
    for text in ["0", "0.0000", "-1", "-0.00"] {
        err(&b, text, BoundError::Sign);
    }
    ok(&b, "0.0001", "0.0001");
}

#[test]
fn non_negative_accepts_zero_and_refuses_negatives_as_sign() {
    ok(&N20_2, "0", "0");
    ok(&N20_2, "0.00", "0");
    err(&N20_2, "-1", BoundError::Sign);
    err(&N20_2, "-0.00", BoundError::Sign);
}

#[test]
fn signed_accepts_a_leading_minus_and_never_a_plus() {
    let b = Bound::numeric(9, 4, Sign::Signed);
    ok(&b, "-1.25", "-1.25");
    ok(&b, "-0.00", "0");
    ok(&b, "3", "3");
    err(&b, "+3", BoundError::Syntax);
}

// ---------------------------------------------------------------- magnitude: numeric(p, s) is strict

#[test]
fn numeric_20_2_takes_everything_below_ten_to_the_18() {
    ok(&N20_2, "999999999999999999.99", "999999999999999999.99");
    err(&N20_2, "1000000000000000000", BoundError::OutOfRange);
    err(&N20_2, "1000000000000000000.00", BoundError::OutOfRange);
}

#[test]
fn numeric_9_4_signed_is_strict_on_both_sides() {
    let b = Bound::numeric(9, 4, Sign::Signed);
    ok(&b, "99999.9999", "99999.9999");
    ok(&b, "-99999.9999", "-99999.9999");
    err(&b, "100000", BoundError::OutOfRange);
    err(&b, "-100000", BoundError::OutOfRange);
}

#[test]
fn numeric_20_8_takes_everything_below_ten_to_the_12() {
    let b = Bound::numeric(20, 8, Sign::Positive);
    ok(&b, "999999999999.99999999", "999999999999.99999999");
    err(&b, "1000000000000", BoundError::OutOfRange);
}

#[test]
fn a_number_too_long_for_decimal_is_out_of_range_not_syntax() {
    // 40 integer digits: past what `Decimal` holds. It is a well-formed number that is too large.
    let huge = "1".repeat(40);
    err(&N20_2, &huge, BoundError::OutOfRange);
    err(
        &Bound::numeric(9, 4, Sign::Signed),
        &format!("-{huge}"),
        BoundError::OutOfRange,
    );
}

// ---------------------------------------------------------------- at_most is inclusive (design :22, :59-61)

#[test]
fn at_most_accepts_the_maximum_and_refuses_one_hundredth_above() {
    let b = Bound::numeric(20, 2, Sign::NonNegative).at_most(100_000_000);
    ok(&b, "100000000", "100000000");
    ok(&b, "100000000.00", "100000000");
    err(&b, "100000000.01", BoundError::OutOfRange);
    err(&b, "100000001", BoundError::OutOfRange);
}

#[test]
fn at_most_applies_to_the_absolute_value_under_signed() {
    let b = Bound::numeric(20, 2, Sign::Signed).at_most(500);
    ok(&b, "-500", "-500");
    err(&b, "-500.01", BoundError::OutOfRange);
}

// ---------------------------------------------------------------- order of refusals

#[test]
fn syntax_is_reported_before_scale_sign_or_range() {
    err(
        &Bound::numeric(20, 2, Sign::Positive),
        "-1.234x",
        BoundError::Syntax,
    );
}

// ---------------------------------------------------------------- IntBound (B-7)

#[test]
fn int_bound_mirrors_a_between_check_on_both_sides() {
    let b = IntBound::integer(5, 240);
    assert_eq!(b.check(5), Ok(5));
    assert_eq!(b.check(240), Ok(240));
    assert_eq!(b.check(4), Err(BoundError::OutOfRange));
    assert_eq!(b.check(241), Err(BoundError::OutOfRange));
}

#[test]
fn int_bound_mirrors_greater_than_zero_and_greater_or_equal_zero() {
    let positive = IntBound::integer(1, i64::MAX);
    assert_eq!(positive.check(1), Ok(1));
    assert_eq!(positive.check(i64::MAX), Ok(i64::MAX));
    assert_eq!(positive.check(0), Err(BoundError::OutOfRange));
    let non_negative = IntBound::integer(0, i64::MAX);
    assert_eq!(non_negative.check(0), Ok(0));
    assert_eq!(non_negative.check(-1), Err(BoundError::OutOfRange));
    assert_eq!(non_negative.check(i64::MIN), Err(BoundError::OutOfRange));
}
