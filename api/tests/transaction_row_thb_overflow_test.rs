//! #2063 floor: `TransactionRow` computes its two THB figures with the checked
//! `ExchangeArithmetic::try_thb`, and a stored row whose figure cannot be held at 2 decimal
//! places answers `AppError::Internal` naming the transaction, as a corrupt column does.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The two sites (`transaction_row.rs:219` principal, `:235` reference value) call the
//! unchecked `ExchangeArithmetic::thb`, which since #3030 panics with "not representable at 2
//! decimal places" on such a figure. `TransactionRow::from_sql_row` already returns
//! `Result<Self, AppError>` and answers `AppError::Internal("invalid … in transaction <ref>")`
//! for every column it cannot read (`transaction_row.rs:106-149`); after this ticket a figure
//! it cannot compute is answered the same way instead of panicking on the request's worker.
//!
//! The figures used:
//! - `quantity numeric(20,4)` holds at most `9999999999999999.9999`; `Rate::parse` caps a rate
//!   at 1e8 (`rate.rs`); `UnitSize::parse` allows `0.0001`. So 9999999999999999.9999 × 1e8 ÷
//!   0.0001 ≈ 10^28 THB, past the widest figure at 2 places (2^96 − 1 hundredths ≈ 7.9 × 10^26).
//! - The same quantity at rate 1 per 0.0001 is 99999999999999999999.00 THB, which fits.
//!
//! Each Err row is built so that only one site overflows, so reverting that one site to the
//! unchecked `thb` turns that one test red.
//!
//! Signatures assumed (all on `main` today): `TransactionRow::from_sql_row(TxSqlRow) ->
//! Result<TransactionRow, AppError>`, `TransactionRow::from_record(&TransactionRecord) ->
//! TransactionRow` (pinned `[required]` in `rates_row_precision_781_test.rs:44`) and
//! `TxSqlRow::try_into_record`.

use std::panic::{AssertUnwindSafe, catch_unwind};

use chrono::{NaiveDate, Utc};
use pos_error::AppError;
use uuid::Uuid;
use xchange::presentation::dto::v3::transaction_row::{TransactionRow, TxSqlRow};

const DOC_REF: &str = "XC-20261018-2063";
const MAX_QUANTITY: &str = "9999999999999999.9999";
const RATE_CAP: &str = "100000000.00000000";
const SMALLEST_UNIT: &str = "0.0001";

fn row(side: &str, amount: &str, rate: &str, reference_rate: Option<&str>, unit: &str) -> TxSqlRow {
    TxSqlRow {
        id: Uuid::nil(),
        status: "COMPLETED".into(),
        cancel_reason: None,
        document_ref: Some(DOC_REF.into()),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 18).expect("a date"),
        recorded_at: Utc::now(),
        r#type: side.into(),
        code: "USD".into(),
        amount: amount.into(),
        rate: rate.into(),
        reference_rate: reference_rate.map(Into::into),
        unit_size: unit.into(),
        fee: "0.00".into(),
        bank_fee: "0.00".into(),
        principal: "0.00".into(),
        settlement: "0.00".into(),
        method: "Cash".into(),
        payment_ref: "".into(),
        customer_id: "C-2063".into(),
        customer_name: "Customer 2063".into(),
        staff_id: "EMP-1".into(),
        notes: "".into(),
        shift_id: None,
        slip_key: None,
    }
}

/// Runs `from_sql_row` and requires `Err(AppError::Internal(msg))` naming the transaction:
/// never a panic, never `Ok`.
fn assert_internal_error_naming_the_transaction(r: TxSqlRow, which: &str) {
    let outcome = catch_unwind(AssertUnwindSafe(|| TransactionRow::from_sql_row(r)));
    match outcome {
        Err(panic) => {
            let text = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            panic!("{which}: from_sql_row panicked instead of answering an error: {text}");
        }
        Ok(Ok(row)) => panic!(
            "{which}: a figure too wide for 2 places came back as a row: principal={:?} \
             referenceValue={:?}",
            row.principal, row.reference_value
        ),
        Ok(Err(AppError::Internal(msg))) => {
            assert!(
                msg.contains(DOC_REF),
                "{which}: the error must name the transaction {DOC_REF}, got: {msg}"
            );
        }
        Ok(Err(other)) => panic!("{which}: expected AppError::Internal, got {other:?}"),
    }
}

// ── Err rows ────────────────────────────────────────────────────────────────

/// Site `transaction_row.rs:219` (principal): quantity max × rate cap ÷ 0.0001 ≈ 10^28 THB.
/// No reference rate, so the reference site is never reached.
#[test]
fn a_row_whose_principal_is_too_wide_answers_an_internal_error() {
    let r = row("BUY", MAX_QUANTITY, RATE_CAP, None, SMALLEST_UNIT);
    assert_internal_error_naming_the_transaction(r, "principal site");
}

/// Site `transaction_row.rs:235` (reference value): the principal fits (rate 1 ⇒
/// 99999999999999999999.00 THB) and only the reference rate at the cap is too wide.
#[test]
fn a_row_whose_reference_value_is_too_wide_answers_an_internal_error() {
    let r = row("SELL", MAX_QUANTITY, "1", Some(RATE_CAP), SMALLEST_UNIT);
    assert_internal_error_naming_the_transaction(r, "reference site");
}

/// `from_record` keeps its `-> Self` signature (pinned in `rates_row_precision_781_test.rs:44`).
/// On a figure it cannot compute it must still refuse rather than return a row: a row with a
/// zero, a ÷10/÷100 or any other figure is a FAIL. Green on `main` (it panics there).
#[test]
fn from_record_never_returns_a_row_for_a_figure_too_wide_for_two_places() {
    let record = row("BUY", MAX_QUANTITY, RATE_CAP, Some(RATE_CAP), SMALLEST_UNIT)
        .try_into_record()
        .expect("every column reads");
    let outcome = catch_unwind(AssertUnwindSafe(|| TransactionRow::from_record(&record)));
    if let Ok(row) = outcome {
        panic!(
            "from_record returned a row for a ≈10^28 THB figure: principal={:?} \
             referenceValue={:?}",
            row.principal, row.reference_value
        );
    }
}

// ── kept rows ───────────────────────────────────────────────────────────────

/// The widest figure a stored row can hold today (quantity max at rate 1 per 0.0001) is
/// computed exactly, at both sites. Green on `main`; must stay green.
#[test]
fn the_widest_figure_a_stored_row_holds_is_computed_exactly() {
    let record = row("BUY", MAX_QUANTITY, "1", Some("1"), SMALLEST_UNIT)
        .try_into_record()
        .expect("every column reads");
    let r = TransactionRow::from_record(&record);
    assert_eq!(r.principal, "99999999999999999999.00");
    assert_eq!(r.settlement, "99999999999999999999.00");
    assert_eq!(
        r.reference_value.as_deref(),
        Some("99999999999999999999.00")
    );
    assert_eq!(r.reference_spread.as_deref(), Some("0.00"));
    assert_eq!(r.reference_percent.as_deref(), Some("0.000000"));

    let via_sql =
        TransactionRow::from_sql_row(row("BUY", MAX_QUANTITY, "1", Some("1"), SMALLEST_UNIT))
            .expect("a row that fits");
    assert_eq!(
        via_sql.reference_value.as_deref(),
        Some("99999999999999999999.00")
    );
}

/// An ordinary BUY with a fee: every computed figure is what it is today. Rounding is HALF_UP
/// to 2 places once (100.5 × 32.123 = 3228.3615 ⇒ 3228.36; × 32.5 = 3266.25); spread 37.89 ÷
/// 3266.25 × 100 = 1.160046 (6 places).
#[test]
fn an_ordinary_buy_row_keeps_every_figure() {
    let mut sql = row("BUY", "100.5", "32.123", Some("32.5"), "1");
    sql.fee = "10.00".into();
    let record = sql.try_into_record().expect("every column reads");
    let r = TransactionRow::from_record(&record);
    assert_eq!(r.principal, "3228.36");
    assert_eq!(r.settlement, "3218.36");
    assert_eq!(r.reference_value.as_deref(), Some("3266.25"));
    assert_eq!(r.reference_spread.as_deref(), Some("37.89"));
    assert_eq!(r.reference_percent.as_deref(), Some("1.160046"));
}

/// An ordinary SELL with no reference rate: principal + fee, and no reference fields.
#[test]
fn an_ordinary_sell_row_without_a_reference_rate_keeps_every_figure() {
    let mut sql = row("SELL", "250", "0.2345", None, "100");
    sql.fee = "1.50".into();
    let record = sql.try_into_record().expect("every column reads");
    let r = TransactionRow::from_record(&record);
    // 250 × 0.2345 ÷ 100 = 0.58625 ⇒ 0.59 (HALF_UP).
    assert_eq!(r.principal, "0.59");
    assert_eq!(r.settlement, "2.09");
    assert_eq!(r.reference_value, None);
    assert_eq!(r.reference_spread, None);
    assert_eq!(r.reference_percent, None);
}
