//! Up-front tests for CR-c1 (#5119): `TransactionRow` reads the stored `principal` and
//! `settlement` from `TransactionRecord` instead of recomputing them from
//! `amount × rate ÷ unit_size ∓ fee`.

use chrono::{NaiveDate, Utc};
use uuid::Uuid;
use xchange::presentation::dto::v3::transaction_row::{TransactionRow, TxSqlRow};

fn sql_row(side: &str, amount: &str, rate: &str, principal: &str, settlement: &str) -> TxSqlRow {
    TxSqlRow {
        id: Uuid::nil(),
        status: "COMPLETED".into(),
        cancel_reason: None,
        document_ref: Some("XC-20261026-5119".into()),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 26).expect("a date"),
        recorded_at: Utc::now(),
        r#type: side.into(),
        code: "USD".into(),
        amount: amount.into(),
        rate: rate.into(),
        reference_rate: Some("32.5000".into()),
        unit_size: "1".into(),
        fee: "0.00".into(),
        bank_fee: "0.00".into(),
        principal: principal.into(),
        settlement: settlement.into(),
        method: "Cash".into(),
        payment_ref: "".into(),
        customer_id: "C-5119".into(),
        customer_name: "Customer 5119".into(),
        staff_id: "EMP-1".into(),
        notes: "".into(),
        shift_id: None,
        slip_key: None,
        backdated: false,
    }
}

/// R1: `TransactionRow` from a record whose stored `principal` 32.10 / `settlement` 32.25 differ
/// from `quantity × rate` (1 × 32.10) for BUY has `principal` `"32.10"` and `settlement` `"32.25"`.
#[test]
fn r1_buy_row_from_record_uses_stored_principal_and_settlement() {
    let record = sql_row("BUY", "1.00", "32.1000", "32.10", "32.25")
        .try_into_record()
        .expect("valid record");
    let row = TransactionRow::from_record(&record);
    assert_eq!(row.principal, "32.10");
    assert_eq!(row.settlement, "32.25");

    let benefit_record = sql_row("BUY", "1.00", "32.5000", "32.10", "32.25")
        .try_into_record()
        .expect("valid record");
    let benefit_row = TransactionRow::from_record(&benefit_record);
    assert_eq!(benefit_row.principal, "32.10");
    assert_eq!(benefit_row.settlement, "32.25");
}

/// R2: the same for SELL, with stored `settlement` 32.00: `settlement` is `"32.00"`.
#[test]
fn r2_sell_row_from_record_uses_stored_principal_and_settlement() {
    let record = sql_row("SELL", "1.00", "32.1000", "32.10", "32.00")
        .try_into_record()
        .expect("valid record");
    let row = TransactionRow::from_record(&record);
    assert_eq!(row.principal, "32.10");
    assert_eq!(row.settlement, "32.00");
}

/// R3: a record with stored principal 3210.00 / settle 3210.00 (consistent) produces unchanged
/// output, as `k2063 …ordinary_transaction_row_succeeds_unchanged` pins today.
#[test]
fn r3_consistent_record_produces_unchanged_output() {
    let sql = sql_row("BUY", "100.00", "32.1000", "3210.00", "3210.00");
    let record = sql.clone().try_into_record().expect("valid record");
    let from_rec = TransactionRow::from_record(&record);
    assert_eq!(from_rec.principal, "3210.00");
    assert_eq!(from_rec.settlement, "3210.00");
    assert_eq!(from_rec.reference_rate.as_deref(), Some("32.50000000"));
    assert_eq!(from_rec.reference_value.as_deref(), Some("3250.00"));
    assert_eq!(from_rec.reference_spread.as_deref(), Some("40.00"));

    let from_sql = TransactionRow::from_sql_row(sql).expect("valid sql row");
    assert_eq!(from_sql.principal, "3210.00");
    assert_eq!(from_sql.settlement, "3210.00");
}
