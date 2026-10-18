//! #3029: a stored transaction line whose THB figure is too wide for 2 decimal places never comes
//! back as a row with that figure ÷100.
//! Written before the implementation by the ticket author (claude/dell-claude-4). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `TransactionRow::from_record` returns `Self` (its signature is pinned in
//! `rates_row_precision_781_test.rs:44`), so after the kernel guard the unchecked `thb` it calls
//! panics. `from_sql_row` therefore either returns an error or panics; it must never return
//! `Ok` with a mis-scaled figure.

use std::panic::{AssertUnwindSafe, catch_unwind};

use chrono::{NaiveDate, Utc};
use uuid::Uuid;
use xchange::presentation::dto::v3::transaction_row::{TransactionRow, TxSqlRow};

fn row(amount: &str, rate: &str, unit_size: &str) -> TxSqlRow {
    TxSqlRow {
        id: Uuid::nil(),
        status: "COMPLETED".into(),
        cancel_reason: None,
        document_ref: Some("XC-20261017-3029".into()),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 17).expect("a date"),
        recorded_at: Utc::now(),
        r#type: "BUY".into(),
        code: "USD".into(),
        amount: amount.into(),
        rate: rate.into(),
        reference_rate: Some(rate.into()),
        unit_size: unit_size.into(),
        fee: "0.00".into(),
        bank_fee: "0.00".into(),
        principal: "0.00".into(),
        settlement: "0.00".into(),
        method: "Cash".into(),
        payment_ref: "".into(),
        customer_id: "C-1".into(),
        customer_name: "Customer One".into(),
        staff_id: "EMP-1".into(),
        notes: "".into(),
        shift_id: None,
        slip_key: None,
    }
}

#[test]
fn a_figure_too_wide_for_two_places_never_comes_back_as_a_row() {
    // quantity numeric(20,4) max × the Rate::parse cap (1e8) ÷ unit_size 0.0001 ≈ 10^28 THB.
    // Today this returns Ok with reference_value 99999999999999999999000000.00 (÷100).
    let worst = row("9999999999999999.9999", "100000000.00000000", "0.0001");
    let outcome = catch_unwind(AssertUnwindSafe(|| TransactionRow::from_sql_row(worst)));
    if let Ok(Ok(r)) = outcome {
        panic!(
            "a too-wide figure came back as a row: referenceValue={:?}",
            r.reference_value
        );
    }
}

#[test]
fn an_ordinary_row_is_unchanged() {
    let r = TransactionRow::from_sql_row(row("100.00", "32.1000", "1")).expect("an ordinary row");
    assert_eq!(r.reference_value.as_deref(), Some("3210.00"));
}
