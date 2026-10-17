//! Tests for #2865 / #2063 tail: admin_shifts.rs uses MoneyDelta::minor() (an i128, cannot wrap).
//!
//! Per ticket #2865 §8:
//! "a K-b site that is MoneyDelta::minor() (an i128, cannot wrap) | leave it unchanged, and say so in the PR: it is part of the ratchet's floor"

use xc_kernel::money::{Money, MoneyDelta};

#[test]
fn test_money_delta_minor_does_not_wrap() {
    let large_positive = (i64::MAX as i128) + 1_000_000;
    let delta = MoneyDelta::from_minor(large_positive);
    assert_eq!(delta.minor(), large_positive);

    let large_negative = (i64::MIN as i128) - 1_000_000;
    let delta_neg = MoneyDelta::from_minor(large_negative);
    assert_eq!(delta_neg.minor(), large_negative);
}

#[test]
fn test_admin_shifts_balance_expected_computation_with_large_values() {
    // In admin_shifts.rs:188:
    // MoneyDelta::from_minor(c.minor_i128() - d.minor())
    // where c is Money and d is MoneyDelta.
    // If d exceeds i64::MAX hundredths, d.minor() does not wrap past i64::MAX because it returns i128.
    let c = Money::from_minor(100).expect("valid money");
    let d = MoneyDelta::from_minor((i64::MAX as i128) + 500);

    let expected = MoneyDelta::from_minor(c.minor_i128() - d.minor());
    assert_eq!(expected.minor(), 100 - ((i64::MAX as i128) + 500));
}
