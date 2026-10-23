//! Tests for #2856 / #2063 tail: custody_repository.rs uses MoneyDelta::minor() (an i128, cannot wrap).
//!
//! Per ticket #2856 §8:
//! "a K-b site that is MoneyDelta::minor() (an i128, cannot wrap) | leave it unchanged, and say so in the PR: it is part of the ratchet's floor"

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, MoneyDelta, QuantityDelta};
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;

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
fn test_stock_delta_preserves_large_i128_delta() {
    let usd = CurrencyCode::parse("USD").expect("valid currency");
    let large_positive = (i64::MAX as i128) + 500_000;
    let delta =
        QuantityDelta::from_decimal(Decimal::from_i128_with_scale(large_positive, 2)).unwrap();
    let stock_delta = StockDelta::of(&[(usd, delta)]);

    let non_zero = stock_delta.non_zero();
    assert_eq!(non_zero.len(), 1);
    assert_eq!(non_zero[0].0, usd);
    assert_eq!(
        non_zero[0].1.value(),
        Decimal::from_i128_with_scale(large_positive, 2)
    );
}
