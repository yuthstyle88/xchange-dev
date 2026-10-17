//! The reference percent rounds the same way on every screen (BE controller's ruling, claude/7ca80c, 2026-10-15).
//!
//! Every money or percent display rounds through the kernel (`xc_kernel::money`: `round_cents`/`fixed2`/`fixed6`),
//! v2's `Math.round` rule: a half goes toward +∞. Nothing rounds with `MidpointAwayFromZero` or `round_dp`.
//! Reports already does this (`views/admin_reports.rs:333`). Cash (`views/admin_cash.rs:61-67`) and the
//! transaction row (`dto/v3/transaction_row.rs:243-250`) use `MidpointAwayFromZero`, so a negative spread exactly on
//! a half shows −0.13 on Cash where Reports shows −0.12.
//!
//! Written before the implementation; do not edit, delete or weaken. No database is needed.
//! (#2815 changed only the call shape: `compute_reference_percent` now returns `AppResult<String>`, so each row
//! compares `.as_deref().ok()` with `Some(..)`. Every expected value is unchanged.)
//!
//! Only a **negative** half, or a value that rounds to zero from below, can tell the two rules apart: a positive half
//! and an exact value come out the same under both. Those rows are pins (green on main) and stay green after the fix.

use std::str::FromStr;

use chrono::{NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use xc_kernel::money::{Rate, UnitSize};
use xchange::presentation::dto::v3::transaction_row::{TransactionRecord, TransactionRow};
use xchange::presentation::http::v3::admin_cash::compute_reference_percent;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{s} is a decimal: {e}"))
}

// ── Cash: two places (`compute_reference_percent(spread, total reference value)`) ─────────────

/// −1.25 / 1000 × 100 = −0.125 % exactly: kernel −0.12; `MidpointAwayFromZero` −0.13. Red on main.
#[test]
fn cash_a_negative_half_rounds_toward_plus_infinity() {
    assert_eq!(
        compute_reference_percent(d("-1.25"), d("1000"))
            .as_deref()
            .ok(),
        Some("-0.12")
    );
}

/// −23.45 / 1000 × 100 = −2.345 %: kernel −2.34; `MidpointAwayFromZero` −2.35. Red on main.
#[test]
fn cash_another_negative_half_rounds_toward_plus_infinity() {
    assert_eq!(
        compute_reference_percent(d("-23.45"), d("1000"))
            .as_deref()
            .ok(),
        Some("-2.34")
    );
}

/// −0.04 / 1000 × 100 = −0.004 %: rounds to zero, shown `0.00`, never `-0.00` (`fixed2`).
#[test]
fn cash_a_value_that_rounds_to_zero_is_never_minus_zero() {
    assert_eq!(
        compute_reference_percent(d("-0.04"), d("1000"))
            .as_deref()
            .ok(),
        Some("0.00")
    );
}

/// +0.125 % → 0.13 under both rules (pin).
#[test]
fn cash_a_positive_half_rounds_up() {
    assert_eq!(
        compute_reference_percent(d("1.25"), d("1000"))
            .as_deref()
            .ok(),
        Some("0.13")
    );
}

/// 15 / 1000 × 100 = 1.5 % exactly → 1.50 (pin).
#[test]
fn cash_an_exact_value_is_unchanged() {
    assert_eq!(
        compute_reference_percent(d("15"), d("1000"))
            .as_deref()
            .ok(),
        Some("1.50")
    );
}

/// No reference value → 0.00, as today (pin).
#[test]
fn cash_no_reference_value_is_zero() {
    assert_eq!(
        compute_reference_percent(d("5"), d("0")).as_deref().ok(),
        Some("0.00")
    );
}

// ── The transaction row: six places (`referencePercent`) ─────────────────────────────────────

/// A BUY of `amount` USD at `rate`, reference 32 per 1.
fn buy(amount: &str, rate: &str) -> TransactionRecord {
    TransactionRecord {
        document_ref: "XC-PCT-1".into(),
        recorded_at: Utc.with_ymd_and_hms(2026, 10, 15, 3, 4, 5).unwrap(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
        side: "BUY".into(),
        currency: "USD".into(),
        amount: d(amount),
        rate: Rate::parse(rate).expect("a valid rate"),
        reference_rate: Some(Rate::parse("32").expect("a valid reference rate")),
        unit_size: UnitSize::parse("1").expect("a valid unit size"),
        fee: d("0.00"),
        bank_fee: d("0.00"),
        method: "Cash".into(),
        payment_ref: String::new(),
        customer_id: "CU-PCT".into(),
        customer_name: "Somchai".into(),
        staff: "owner".into(),
        notes: String::new(),
        shift_id: None,
        slip_key: None,
    }
}

fn reference_percent(record: &TransactionRecord) -> Value {
    serde_json::to_value(TransactionRow::from_record(record)).expect("a row serializes")
        ["referencePercent"]
        .clone()
}

/// 2500 USD: reference value 80,000.00; at 32.000004 the principal is 80,000.01, so the spread (BUY: reference −
/// principal) is −0.01 and the percent −0.01 / 80,000 × 100 = −0.0000125 exactly. Kernel `fixed6` −0.000012;
/// `MidpointAwayFromZero` −0.000013. Red on main.
#[test]
fn row_a_negative_half_rounds_toward_plus_infinity() {
    assert_eq!(
        reference_percent(&buy("2500", "32.000004")),
        Value::from("-0.000012")
    );
}

/// At 31.999996 the principal is 79,999.99: spread +0.01, percent +0.0000125 → 0.000013 under both rules (pin).
#[test]
fn row_a_positive_half_rounds_up() {
    assert_eq!(
        reference_percent(&buy("2500", "31.999996")),
        Value::from("0.000013")
    );
}

/// At 32.1 the principal is 80,250.00: spread −250.00, percent −0.3125 exactly → −0.312500 (pin).
#[test]
fn row_an_exact_value_is_unchanged() {
    assert_eq!(
        reference_percent(&buy("2500", "32.1")),
        Value::from("-0.312500")
    );
}
