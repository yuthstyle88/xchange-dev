//! Tests for #2863 / #2063 tail: alert_facts.rs uses MoneyDelta::minor() (an i128, cannot wrap).
//!
//! Per ticket #2863 §8:
//! "a K-b site that is MoneyDelta::minor() (an i128, cannot wrap) | leave it unchanged, and say so in the PR: it is part of the ratchet's floor"

use xc_kernel::money::MoneyDelta;

#[test]
fn test_money_delta_minor_does_not_wrap() {
    let large_positive = (i64::MAX as i128) + 1_000_000;
    let delta = MoneyDelta::from_minor(large_positive);
    assert_eq!(delta.minor(), large_positive);

    let large_negative = (i64::MIN as i128) - 1_000_000;
    let delta_neg = MoneyDelta::from_minor(large_negative);
    assert_eq!(delta_neg.minor(), large_negative);
}
