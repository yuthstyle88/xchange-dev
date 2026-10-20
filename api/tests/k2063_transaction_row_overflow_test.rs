//! Tests for #2866 / #2063 tail: `TransactionRow::from_record` arithmetic.
//!
//! Per controller ruling (#2866 issuecomment-6241814822, review issuecomment-6242058089) and #2063 bounds rule (issuecomment-6231704691):
//! Both sites in `TransactionRow::from_record` (`transaction_row.rs:218` and `:233`) compute display
//! fields from a stored `TransactionRecord` whose fields are read back from PostgreSQL and bounded:
//! - `Rate::parse` caps `rate` and `reference_rate` at `Money::MAX_INPUT_UNITS` (`rate.rs:20`, applied at `transaction_row.rs:117` and `:124`);
//! - `quantity numeric(20,4)` (`api/migrations/20261111170000_transaction_states.sql:81`) and `UnitSize::parse >= 0.0001` bound the rest;
//! - so the raw value stays at or below about 1e28, below `Decimal::MAX`.
//!
//! Arithmetic in `ExchangeArithmetic::thb` operates in `Decimal` and `i128` (`.minor_i128()`), which
//! cannot overflow. These are K-b sites that cannot overflow.
//! Earlier bounds in `TransactionRecord::try_from` reject invalid or out-of-bounds rates with `AppError::Internal`.

use chrono::{NaiveDate, Utc};
use pos_error::AppError;
use uuid::Uuid;
use xchange::presentation::dto::v3::transaction_row::{TransactionRow, TxSqlRow};

fn sample_sql_row() -> TxSqlRow {
    TxSqlRow {
        id: Uuid::nil(),
        status: "COMPLETED".into(),
        cancel_reason: None,
        document_ref: Some("XC-20261017-0001".into()),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
        recorded_at: Utc::now(),
        r#type: "BUY".into(),
        code: "USD".into(),
        amount: "100.00".into(),
        rate: "32.1000".into(),
        reference_rate: Some("32.5000".into()),
        unit_size: "1".into(),
        fee: "0.00".into(),
        bank_fee: "0.00".into(),
        principal: "3210.00".into(),
        settlement: "3210.00".into(),
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
fn ordinary_transaction_row_succeeds_unchanged() {
    let row = sample_sql_row();
    let res = TransactionRow::from_sql_row(row).expect("valid SQL row decodes to TransactionRow");
    assert_eq!(res.principal, "3210.00");
    assert_eq!(res.settlement, "3210.00");
    assert_eq!(res.reference_rate.as_deref(), Some("32.50000000"));
}

#[test]
fn earlier_bound_rejects_rate_exceeding_max_input_units() {
    // Rate::parse caps rate and reference_rate at Money::MAX_INPUT_UNITS (100_000_000).
    // A stored rate of "100000000.00000001" makes TransactionRow::from_sql_row return AppError::Internal whose text starts "invalid rate".
    let mut row_rate = sample_sql_row();
    row_rate.rate = "100000000.00000001".into();
    let err = TransactionRow::from_sql_row(row_rate).unwrap_err();
    match err {
        AppError::Internal(msg) => {
            assert!(
                msg.starts_with("invalid rate"),
                "expected error text starting with 'invalid rate', got: {msg}"
            );
        }
        other => panic!("expected AppError::Internal, got: {other:?}"),
    }

    // A stored reference_rate of "100000000.00000001" makes TransactionRow::from_sql_row return AppError::Internal whose text starts "invalid reference_rate".
    let mut row_ref_rate = sample_sql_row();
    row_ref_rate.reference_rate = Some("100000000.00000001".into());
    let err = TransactionRow::from_sql_row(row_ref_rate).unwrap_err();
    match err {
        AppError::Internal(msg) => {
            assert!(
                msg.starts_with("invalid reference_rate"),
                "expected error text starting with 'invalid reference_rate', got: {msg}"
            );
        }
        other => panic!("expected AppError::Internal, got: {other:?}"),
    }
}

#[test]
fn earlier_bound_rejects_corrupted_inputs_with_app_error_internal() {
    let mut bad_amount = sample_sql_row();
    bad_amount.amount = "invalid_numeric".into();
    let err = TransactionRow::from_sql_row(bad_amount).unwrap_err();
    assert!(
        matches!(err, AppError::Internal(_)),
        "earlier bound must reject corrupt amount with AppError::Internal, got {err:?}"
    );

    let mut bad_rate = sample_sql_row();
    bad_rate.rate = "-5.00".into();
    let err = TransactionRow::from_sql_row(bad_rate).unwrap_err();
    assert!(
        matches!(err, AppError::Internal(_)),
        "earlier bound must reject negative rate with AppError::Internal, got {err:?}"
    );

    let mut bad_unit = sample_sql_row();
    bad_unit.unit_size = "0".into();
    let err = TransactionRow::from_sql_row(bad_unit).unwrap_err();
    assert!(
        matches!(err, AppError::Internal(_)),
        "earlier bound must reject zero unit_size with AppError::Internal, got {err:?}"
    );
}
