//! #2815 (parent #2108): Cash's reference percent must fail, not show `0.00`, when it is too large to round to cents.
//!
//! `compute_reference_percent` (`api/src/presentation/http/v3/views/admin_cash.rs`) rounds through the kernel's
//! `round_cents`, which answers `None` above the 96-bit range. On main that `None` becomes `unwrap_or(0)` → `"0.00"`.
//! The fix returns `AppResult<String>` and answers `AppError::Internal` there, like its caller already does for the
//! spread (`round_cents(total_spread).ok_or_else(..)`).
//!
//! Written before the implementation; do not edit, delete or weaken. No database is needed.

use std::str::FromStr;

use pos_error::AppError;
use rust_decimal::Decimal;
use xchange::presentation::http::v3::admin_cash::compute_reference_percent;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{s} is a decimal: {e}"))
}

/// 1e26 / 1 × 100 = 1e28 %: 1e30 cents is above the 96-bit range. Main shows "0.00".
#[test]
fn a_percent_above_the_cents_range_is_an_internal_error() {
    let got = compute_reference_percent(d("100000000000000000000000000"), d("1"));
    assert!(
        matches!(got, Err(AppError::Internal(_))),
        "expected AppError::Internal, got {got:?}"
    );
}

/// The same below zero. Main shows "0.00".
#[test]
fn a_negative_percent_below_the_cents_range_is_an_internal_error() {
    let got = compute_reference_percent(d("-100000000000000000000000000"), d("1"));
    assert!(
        matches!(got, Err(AppError::Internal(_))),
        "expected AppError::Internal, got {got:?}"
    );
}

/// 7e24 / 1 × 100 = 7e26 % → 7e28 cents, still inside the range: shown in full (pin).
#[test]
fn a_large_percent_inside_the_cents_range_is_shown() {
    let got = compute_reference_percent(d("7000000000000000000000000"), d("1"));
    assert_eq!(got.as_deref().ok(), Some("700000000000000000000000000.00"));
}

/// No reference value → "0.00", as today (pin; this zero is the defined answer, not a fallback).
#[test]
fn no_reference_value_is_still_zero() {
    assert_eq!(
        compute_reference_percent(d("5"), d("0")).as_deref().ok(),
        Some("0.00")
    );
}
