//! Ticket #2792 — `TransactionForm::parse` and `TransactionForm::from_transaction` must never
//! silently fall back to `Rate::parse("1")` or `None` when a per-1 rate is out of `Rate`'s range.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::{
    BAD_NUMBER, TOO_MANY_DECIMALS, TransactionForm,
};
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Quote, Rate, Side, UnitSize};

fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

fn internal_error<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Internal(msg)) => msg,
        other => panic!("expected Err(AppError::Internal(<msg>)), got {other:?}"),
    }
}

fn krw_at(rate: &str) -> SaveTransaction {
    SaveTransaction {
        customer: ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: "1990-01-31".to_owned(),
            passport_expiry: "2031-12-31".to_owned(),
            mode: "LIVE".to_owned(),
        },
        submission_id: "sub-0001-aaaaaaaaaaaa".to_owned(),
        customer_id: "CX-00000001".to_owned(),
        side: "BUY".to_owned(),
        currency: "KRW".to_owned(),
        amount: "100000".to_owned(),
        rate: rate.to_owned(),
        fee: "0".to_owned(),
        bank_fee: "0".to_owned(),
        method: "Cash".to_owned(),
        payment_ref: String::new(),
        account: "Cash drawer 1".to_owned(),
        staff: "EMP-001".to_owned(),
        booking: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: "2026-10-06".to_owned(),
        paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
        payment_confirmed: true,
        identity_confirmed: true,
        passport: Some("11111111-1111-4111-8111-111111111111".to_owned()),
        slip: None,
        reference_rate: None,
    }
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap()
}

fn paid_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 5, 30, 0).unwrap()
}

fn sample_transaction() -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    ExchangeTransaction {
        id,
        organization_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        branch_id: BranchId::new("B2".to_owned()),
        mode: Mode::Live,
        status: TransactionStatus::Confirmed,
        document_ref: None,
        quote_id: Some(Uuid::new_v4()),
        booking_id: None,
        shift_id: Some("SH-B2".to_owned()),
        submission_id: "sub-tx4-0000000001".to_owned(),
        payload_hash: "hash".to_owned(),
        customer_id: "CU-000123".to_owned(),
        customer_name: "Somchai Jaidee".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "front".to_owned(),
        booking_no: "BK-42".to_owned(),
        notes: "counted twice".to_owned(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        recorded_at: paid_at(),
        recorded_by: "employee-front".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: Some(Uuid::new_v4()),
            shop_side: Side::Sell,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from(100)),
            unit_size: UnitSize::parse("1").unwrap(),
            rate_snapshot: rate("32.45"),
            reference_rate: Some(rate("32.40")),
            thb_amount: money("3245.00"),
            fee_thb: money("20.00"),
            settle_thb: money("3265.00"),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::BankTransfer,
            reference: "TRX-77".to_owned(),
            account: "KBank 1".to_owned(),
            slip_file_key: Some("22222222-2222-4222-8222-222222222222".to_owned()),
            amount_thb: money("3265.00"),
            bank_fee_thb: money("1.50"),
            paid_at: paid_at(),
        },
        receipts: vec![],
        passport_file_key: "11111111-1111-4111-8111-111111111111".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: true,
    }
}

/// R1: `parse`, `rate = "0.000000001"` (accepted by `CompatRate`, rounds to 0 at 8 places)
/// -> `Err(AppError::Domain(BAD_NUMBER))`.
#[test]
fn r1_parse_rate_rounding_to_zero_at_eight_places_is_refused() {
    assert_eq!(
        refusal(TransactionForm::parse(&krw_at("0.000000001"))),
        BAD_NUMBER
    );
}

/// R2: `parse`, valid `rate`, `reference_rate = "0.000000001"`
/// -> `Err(AppError::Domain(BAD_NUMBER))`.
#[test]
fn r2_parse_reference_rate_rounding_to_zero_at_eight_places_is_refused() {
    let mut cmd = krw_at("0.02512345678");
    cmd.reference_rate = Some("0.000000001".to_owned());
    assert_eq!(refusal(TransactionForm::parse(&cmd)), BAD_NUMBER);
}

/// R3: `parse`, `rate = "0.000000004"` -> `Err(AppError::Domain(BAD_NUMBER))`.
#[test]
fn r3_parse_rate_0_000000004_rounding_to_zero_at_eight_places_is_refused() {
    assert_eq!(
        refusal(TransactionForm::parse(&krw_at("0.000000004"))),
        BAD_NUMBER
    );
}

/// R4: `from_transaction`, a line whose `rate_snapshot ÷ unit_size` is below 0.000000005
/// (e.g. `rate_snapshot 0.00000001`, `unit_size 10`), or above 100,000,000 (fractional `unit_size`),
/// or whose `reference_rate ÷ unit_size` is out of range -> `Err(AppError::Internal(_))` naming the transaction id.
#[test]
fn r4_from_transaction_stored_per_1_rate_out_of_range_fails_with_internal_error() {
    // Sub-threshold rate_snapshot ÷ unit_size (0.00000001 ÷ 10 = 0.000000001)
    let mut t = sample_transaction();
    t.lines[0].rate_snapshot = rate("0.00000001");
    t.lines[0].unit_size = UnitSize::parse("10").unwrap();
    let msg = internal_error(TransactionForm::from_transaction(&t));
    assert!(
        msg.contains(&t.id.0.to_string()),
        "error message must name transaction id {}: got {msg:?}",
        t.id.0
    );

    // Sub-threshold reference_rate ÷ unit_size (0.00000001 ÷ 10 = 0.000000001)
    let mut t_ref = sample_transaction();
    t_ref.lines[0].rate_snapshot = rate("25.0000");
    t_ref.lines[0].unit_size = UnitSize::parse("10").unwrap();
    t_ref.lines[0].reference_rate = Some(rate("0.00000001"));
    let msg_ref = internal_error(TransactionForm::from_transaction(&t_ref));
    assert!(
        msg_ref.contains(&t_ref.id.0.to_string()),
        "error message must name transaction id {}: got {msg_ref:?}",
        t_ref.id.0
    );

    // Above 100,000,000 via fractional unit_size (100,000,000 ÷ 0.5 = 200,000,000)
    let mut t_high = sample_transaction();
    t_high.lines[0].rate_snapshot = rate("100000000");
    t_high.lines[0].unit_size = UnitSize::parse("0.5").unwrap();
    let msg_high = internal_error(TransactionForm::from_transaction(&t_high));
    assert!(
        msg_high.contains(&t_high.id.0.to_string()),
        "error message must name transaction id {}: got {msg_high:?}",
        t_high.id.0
    );
}

/// K1 (kept): `parse`, `rate = "0.02512345678"` (11 places)
/// -> `Ok`; `rate` is the 8-place `Rate` (`0.02512345`), `client_rate` exact.
#[test]
fn k1_parse_11_place_rate_keeps_8_place_rate_and_exact_client_rate() {
    let form = TransactionForm::parse(&krw_at("0.02512345678")).expect("valid 11-place rate");
    assert_eq!(form.rate, rate("0.02512345"));
    assert_eq!(
        form.client_rate,
        "0.02512345678".parse::<Decimal>().unwrap()
    );
}

/// K2 (kept): `parse`, `rate = "32.1"`; `reference_rate = None` / `""`
/// -> `Ok`; `reference_rate None`, `client_reference_rate None`.
#[test]
fn k2_parse_absent_or_empty_reference_rate_gives_none() {
    let mut cmd = krw_at("32.1");
    cmd.reference_rate = None;
    let form_none = TransactionForm::parse(&cmd).expect("None reference_rate");
    assert_eq!(form_none.reference_rate, None);
    assert_eq!(form_none.client_reference_rate, None);

    cmd.reference_rate = Some(String::new());
    let form_empty = TransactionForm::parse(&cmd).expect("empty reference_rate");
    assert_eq!(form_empty.reference_rate, None);
    assert_eq!(form_empty.client_reference_rate, None);
}

/// K3 (kept): `parse`, 12-place rate -> `Err(Domain(TOO_MANY_DECIMALS))`.
#[test]
fn k3_parse_12_place_rate_is_refused_with_too_many_decimals() {
    for r in ["0.025123456781", "32.100000000001"] {
        assert_eq!(
            refusal(TransactionForm::parse(&krw_at(r))),
            TOO_MANY_DECIMALS,
            "{r}"
        );
    }
}

/// K4 (kept): `from_transaction` of an ordinary stored exchange -> identical form to today.
#[test]
fn k4_from_transaction_of_ordinary_stored_exchange_succeeds() {
    let t = sample_transaction();
    let form = TransactionForm::from_transaction(&t).expect("ordinary stored transaction");
    assert_eq!(form.rate, rate("32.45"));
    assert_eq!(form.client_rate, "32.45".parse::<Decimal>().unwrap());
    assert_eq!(form.reference_rate, Some(rate("32.40")));
    assert_eq!(
        form.client_reference_rate,
        Some("32.40".parse::<Decimal>().unwrap())
    );
    assert_eq!(
        form.quote,
        Quote {
            principal: money("3245.00"),
            received: money("3265.00"),
            paid: Money::ZERO,
        }
    );
}
