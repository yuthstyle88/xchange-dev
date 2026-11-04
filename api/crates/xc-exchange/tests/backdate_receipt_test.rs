//! Backdated keying (`docs/design/backdated-keying.md` §8, behaviour table B11; owner rule R4): the receipt of a
//! backdated exchange carries the ย้อนหลัง label; every other receipt prints exactly today's bytes.
//! Written before the implementation by the sub-controller claude/6bb512ce. Do not edit, delete or weaken;
//! add tests in a file of your own. The fixture is copied from `receipt_rounding_test.rs` (#5315).

use chrono::{NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;
use xc_exchange::domain::exchange::entities::receipt::{
    ReceiptMarks, receipt_html, receipt_html_marked,
};
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_exchange::domain::transaction::backdate_mark::BackdateMark;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

const LABEL: &str = "ย้อนหลัง";

fn money(s: &str) -> Money {
    Money::parse(s).expect("a money figure")
}

/// A COMPLETED-to-be stored exchange of one USD line: `thb` principal, `fee`, `settle` (what the drawer moved).
/// 10 USD at 23.555 gives the principal 235.55 used by the B2 case of #5315 §1.
fn stored(
    side: Side,
    rate: &str,
    thb: &str,
    fee: &str,
    settle: &str,
    method: PaymentMethod,
) -> ExchangeTransaction {
    let now = Utc.with_ymd_and_hms(2026, 10, 27, 20, 30, 0).unwrap();
    ExchangeTransaction {
        queue_ticket_id: None,
        id: TransactionId(Uuid::from_u128(0x5315)),
        organization_id: Uuid::from_u128(1),
        branch_id: BranchId::new("MAIN"),
        mode: Mode::Live,
        status: TransactionStatus::Confirmed,
        document_ref: None,
        quote_id: Some(Uuid::from_u128(2)),
        booking_id: None,
        shift_id: Some("SH-1".to_owned()),
        submission_id: "sub-5315".to_owned(),
        payload_hash: "hash".to_owned(),
        customer_id: "CX-00000001".to_owned(),
        customer_name: "Somchai Jaidee".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "37ba9f73-0000-4000-8000-000000000001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(),
        recorded_at: now,
        recorded_by: "37ba9f73-0000-4000-8000-000000000001".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-MAIN".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: true,
        lines: vec![TransactionLine {
            id: Uuid::from_u128(3),
            quote_line_id: None,
            shop_side: side,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from_str("10").unwrap()),
            unit_size: UnitSize::ONE,
            rate_snapshot: Rate::parse(rate).unwrap(),
            reference_rate: None,
            thb_amount: money(thb),
            fee_thb: money(fee),
            settle_thb: money(settle),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::from_u128(4),
            payment_no: 1,
            method,
            reference: if method.is_cash() {
                String::new()
            } else {
                "REF-1".to_owned()
            },
            account: "Cash drawer 1".to_owned(),
            slip_file_key: None,
            amount_thb: money(settle),
            bank_fee_thb: Money::ZERO,
            paid_at: now,
        },
        receipts: vec![],
    }
}

fn form_of(t: &ExchangeTransaction) -> TransactionForm {
    TransactionForm::from_transaction(t).expect("a stored exchange's form")
}

fn form() -> TransactionForm {
    form_of(&stored(
        Side::Buy,
        "32.15",
        "321.50",
        "0.00",
        "321.50",
        PaymentMethod::Cash,
    ))
}

fn mark() -> BackdateMark {
    BackdateMark {
        // 2026-11-05 08:10 ICT
        entered_at: Utc.with_ymd_and_hms(2026, 11, 5, 1, 10, 0).unwrap(),
        entered_by: "owner-1".to_owned(),
    }
}

/// B11: no mark prints today's receipt byte for byte.
#[test]
fn m1_no_mark_is_todays_receipt_byte_for_byte() {
    let f = form();
    assert_eq!(
        receipt_html_marked(&f, "XC-20261104-000001", "XC-B", &ReceiptMarks::default()),
        receipt_html(&f, "XC-20261104-000001", "XC-B")
    );
}

/// B11: a backdated receipt carries the label and the real entry time in Bangkok time.
#[test]
fn m2_a_backdated_receipt_says_so_with_the_entry_time() {
    let html = receipt_html_marked(
        &form(),
        "XC-20261104-000001",
        "XC-B",
        &ReceiptMarks {
            backdated: Some(mark()),
        },
    );
    assert!(html.contains(LABEL), "{html}");
    assert!(html.contains("บันทึกเมื่อ 05/11/2026 08:10"), "{html}");
}

/// B11: an ordinary receipt never says ย้อนหลัง.
#[test]
fn m3_an_ordinary_receipt_never_says_backdated() {
    assert!(!receipt_html(&form(), "XC-20261104-000001", "XC-B").contains(LABEL));
}
