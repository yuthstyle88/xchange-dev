//! Ticket #672 (TX-4), G5 — `TransactionForm::from_transaction`: the form the existing `SaveStep`s
//! are given (D-7) is rebuilt from the stored exchange, so confirm (and TX-5's complete) can run every
//! step unchanged.
//! Written before the implementation (claude/14cc24bd, from a draft by claude/709ec749, on the owner's
//! order, 2026-10-10). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §2 D-7, §4.4 confirm step 3, §4.5, §7
//! (`TransactionForm::from_transaction`); rates design `rates.md` §4.9 R3 (RE-4c #757, on main
//! `6524a80`): the form's `client_rate` is the exact per-1 rate and `rate` is the same value as a
//! `Rate`; the stored `transaction_lines.rate_snapshot` and `reference_rate` are per `unit_size`
//! (#757's ruling: the reference rate is stored per unit too). Ticket #672 G5 and the lane
//! controller's Q2 ruling (issuecomment-6094343896): the exchange stores `passport_file_key`.
//!
//! Locked: every field a registered step reads from `ctx.form` on main
//! (`api/src/application/workflows/{custody,device,kiosk,booking,payment}_save_step.rs`):
//! `customer.mode`, `submission_id`, `customer_id`, `side`, `currency`, `amount`, `rate`, `method`,
//! `booking`; and the rest of the form the exchange can give: the payment fields, `passport`, `slip`,
//! both rates and their exact per-1 values, and `quote` (the stored figures, not recomputed).
//! Not locked (ticket author's choice): `customer.passport_no`, `birth_date`, `passport_expiry`.
//! The exchange stores no plain passport (#554 K4; Q1 = B: no decryption), and no step reads them.
//!
//! No database is used here; nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `TransactionForm::from_transaction(&ExchangeTransaction) -> TransactionForm`, a new
//!   constructor beside `parse` (`transaction_form.rs`, ticket G5). #770 changes only `parse`'s rate
//!   parser; this constructor reads no text.
//! - #670's `ExchangeTransaction`, `TransactionLine`, `TransactionPayment` as pinned in
//!   `api/tests/tx_store_test.rs` (`feat/tx-2-670`).
//! - The form's fields as on main `6524a80` (`transaction_form.rs:27-51`).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Quote, Rate, Side, UnitSize};

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap()
}

fn paid_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, 5, 30, 0).unwrap()
}

/// A CONFIRMED LIVE SELL of 100 USD at 32.45 per 1 (fee 20) by bank transfer with a slip, at B2.
fn usd_sell() -> ExchangeTransaction {
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

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// A DEMO cash BUY of 1,000 JPY at 23.5 per 100 (no fee), walk-in.
fn jpy_buy() -> ExchangeTransaction {
    let mut t = usd_sell();
    t.mode = Mode::Demo;
    t.shift_id = None;
    t.booking_no = "WALK-IN".to_owned();
    t.lines[0] = TransactionLine {
        shop_side: Side::Buy,
        currency: CurrencyCode::parse("JPY").unwrap(),
        quantity: ForeignAmount::from_decimal(Decimal::from(1000)),
        unit_size: UnitSize::parse("100").unwrap(),
        rate_snapshot: rate("23.5"),
        reference_rate: Some(rate("23.4")),
        thb_amount: money("235.00"),
        fee_thb: money("0.00"),
        settle_thb: money("235.00"),
        ..t.lines[0].clone()
    };
    t.payment = TransactionPayment {
        method: PaymentMethod::Cash,
        reference: String::new(),
        account: "Cash drawer 1".to_owned(),
        slip_file_key: None,
        amount_thb: money("235.00"),
        bank_fee_thb: money("0.00"),
        ..t.payment.clone()
    };
    t
}

#[test]
fn every_field_a_save_step_reads_comes_from_the_stored_exchange() {
    let t = usd_sell();
    let form = TransactionForm::from_transaction(&t);

    assert_eq!(
        form.customer.mode,
        Mode::Live,
        "custody/device/kiosk read it"
    );
    assert_eq!(form.submission_id, "sub-tx4-0000000001");
    assert_eq!(form.customer_id, "CU-000123", "the booking step reads it");
    assert_eq!(form.side, Side::Sell, "the line's shop side");
    assert_eq!(form.currency, "USD");
    assert_eq!(form.amount, money("100.00"), "the line's quantity");
    assert_eq!(form.rate, rate("32.45"), "per 1: unit size 1");
    assert_eq!(form.client_rate, dec("32.45"));
    assert_eq!(form.method, PaymentMethod::BankTransfer);
    assert_eq!(form.booking, "BK-42", "the exchange's booking number");
}

#[test]
fn the_payment_fee_and_customer_fields_come_from_the_payment_line_and_exchange() {
    let t = usd_sell();
    let form = TransactionForm::from_transaction(&t);

    assert_eq!(form.fee, money("20.00"), "the line's fee_thb");
    assert_eq!(form.bank_fee, money("1.50"), "the payment's bank fee");
    assert_eq!(form.payment_ref, "TRX-77");
    assert_eq!(form.account, "KBank 1");
    assert_eq!(
        form.slip.as_deref(),
        Some("22222222-2222-4222-8222-222222222222"),
        "the payment's slip file"
    );
    assert_eq!(form.paid_at, paid_at());
    assert_eq!(
        form.value_date,
        NaiveDate::from_ymd_opt(2026, 10, 6).unwrap()
    );
    assert_eq!(form.notes, "counted twice");
    assert_eq!(form.staff, "front", "the exchange's staff_id");
    assert_eq!(form.reference_rate, Some(rate("32.40")));
    assert_eq!(form.client_reference_rate, Some(dec("32.40")));
    assert_eq!(
        form.passport, "11111111-1111-4111-8111-111111111111",
        "Q2: the exchange's passport scan"
    );
    assert_eq!(form.customer.name, "Somchai Jaidee");
    assert_eq!(form.customer.nationality, "THA");
}

/// RE-4c (#757): the form's rate is per 1. 23.5 per 100 is 0.235 per 1, exact.
#[test]
fn a_rate_per_unit_size_becomes_the_forms_per_1_rate() {
    let t = jpy_buy();
    let form = TransactionForm::from_transaction(&t);

    assert_eq!(form.customer.mode, Mode::Demo);
    assert_eq!(form.side, Side::Buy);
    assert_eq!(form.currency, "JPY");
    assert_eq!(form.amount, money("1000.00"));
    assert_eq!(form.rate, rate("0.235"), "rate_snapshot ÷ unit_size");
    assert_eq!(form.client_rate, dec("0.235"), "the exact per-1 rate");
    assert_eq!(form.method, PaymentMethod::Cash);
    assert_eq!(form.slip, None, "cash carries no slip");
    assert_eq!(
        (form.reference_rate, form.client_reference_rate),
        (Some(rate("0.234")), Some(dec("0.234"))),
        "the stored reference is per 100 too (#757): 23.4 per 100 is 0.234 per 1"
    );
    assert_eq!(form.booking, "WALK-IN");
}

/// The constructor reads; the exchange is not changed and two calls agree.
#[test]
fn rebuilding_the_form_twice_gives_the_same_form() {
    let t = usd_sell();
    let before = t.clone();
    assert_eq!(
        TransactionForm::from_transaction(&t),
        TransactionForm::from_transaction(&t)
    );
    assert_eq!(t, before);
}

/// The form's quote is the stored figures (the exchange's own quote, §4.4), not a new computation:
/// principal = `thb_amount`; a SELL takes in `settle_thb`, a BUY pays out `settle_thb`.
#[test]
fn the_forms_quote_is_the_stored_figures_of_the_side() {
    let sell = TransactionForm::from_transaction(&usd_sell());
    assert_eq!(
        sell.quote,
        Quote {
            principal: money("3245.00"),
            received: money("3265.00"),
            paid: Money::ZERO,
        }
    );

    let mut t = jpy_buy();
    t.lines[0].fee_thb = money("5.00");
    t.lines[0].settle_thb = money("230.00");
    let buy = TransactionForm::from_transaction(&t);
    assert_eq!(
        buy.quote,
        Quote {
            principal: money("235.00"),
            received: Money::ZERO,
            paid: money("230.00"),
        }
    );
    assert_eq!(buy.fee, money("5.00"));
}

/// A reference rate that is absent stays absent in both forms of it.
#[test]
fn no_stored_reference_rate_gives_none() {
    let mut t = usd_sell();
    t.lines[0].reference_rate = None;
    let form = TransactionForm::from_transaction(&t);
    assert_eq!(
        (form.reference_rate, form.client_reference_rate),
        (None, None)
    );
}
