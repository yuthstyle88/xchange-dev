//! Backdated keying (`docs/design/backdated-keying.md` §2.1, §5, §10; behaviour table B13): the store writes a
//! backdated draft (LIVE, no shift, `backdated = true`) and reads back who really keyed it.
//! Written before the implementation by the sub-controller claude/6bb512ce. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx4_*` test does.

mod quote_common;
mod tx4_common;

use std::str::FromStr;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use tx4_common::*;
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::backdate_mark::BackdateMark;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

const LIVE_CUSTOMER: &str = "CU-BK-STORE";
const KEYER: &str = "owner-bk";

fn entered() -> DateTime<Utc> {
    // 2026-11-04 21:00 ICT, the real entry
    Utc.with_ymd_and_hms(2026, 11, 4, 14, 0, 0).unwrap()
}

fn paid() -> DateTime<Utc> {
    // 2026-11-04 13:23 ICT, the deal
    Utc.with_ymd_and_hms(2026, 11, 4, 6, 23, 0).unwrap()
}

/// A backdated draft as `BackdateSale` builds it: LIVE, no shift, no quote, BUY 500 USD at 32.15.
fn draft(org: Uuid) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    let m = |s: &str| Money::parse(s).unwrap();
    ExchangeTransaction {
        queue_ticket_id: None,
        id,
        organization_id: org,
        branch_id: BranchId::new("MAIN"),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: None,
        submission_id: id.0.to_string(),
        payload_hash: "hash".to_owned(),
        customer_id: LIVE_CUSTOMER.to_owned(),
        customer_name: "Test Customer".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: KEYER.to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
        recorded_at: entered(),
        recorded_by: KEYER.to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-MAIN".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from_str("500").unwrap()),
            unit_size: UnitSize::ONE,
            rate_snapshot: Rate::parse("32.15").unwrap(),
            reference_rate: None,
            thb_amount: m("16075.00"),
            fee_thb: Money::ZERO,
            settle_thb: m("16075.00"),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::Cash,
            reference: String::new(),
            account: String::new(),
            slip_file_key: None,
            amount_thb: m("16075.00"),
            bank_fee_thb: Money::ZERO,
            paid_at: paid(),
        },
        receipts: vec![],
    }
}

async fn org(desk: &Desk) -> Uuid {
    Uuid::parse_str(
        &desk
            .db
            .text("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
            .await
            .expect("MAIN's organisation"),
    )
    .unwrap()
}

async fn start() -> Desk {
    let desk = Desk::start().await;
    desk.db.customer(LIVE_CUSTOMER, "MAIN").await;
    desk
}

/// S1: a backdated draft is stored LIVE, with no shift, marked `backdated`, at the deal's `paid_at` and the real
/// `recorded_at` / `recorded_by`.
#[actix_web::test]
async fn s1_a_backdated_draft_is_stored_marked_with_no_shift() {
    let desk = start().await;
    let t = draft(org(&desk).await);
    let mut tx = desk.db.pool.begin().await.unwrap();
    desk.store
        .insert_backdated_draft_in(&mut tx, &t)
        .await
        .expect("insert the backdated draft");
    tx.commit().await.unwrap();
    assert_eq!(
        desk.db
            .text(&format!(
                "SELECT concat_ws('|', status, mode, backdated, shift_id IS NULL, recorded_by) \
                 FROM transactions WHERE id = '{}'",
                t.id.0
            ))
            .await
            .as_deref(),
        Some(format!("DRAFT|LIVE|true|true|{KEYER}").as_str())
    );
    desk.db.finish().await;
}

/// S2: the mark reads back the real entry; an ordinary or missing exchange has none.
#[actix_web::test]
async fn s2_the_mark_reads_back_the_real_entry() {
    let desk = start().await;
    let t = draft(org(&desk).await);
    let mut tx = desk.db.pool.begin().await.unwrap();
    desk.store
        .insert_backdated_draft_in(&mut tx, &t)
        .await
        .unwrap();
    assert_eq!(
        desk.store.backdate_mark_in(&mut tx, t.id).await.unwrap(),
        Some(BackdateMark {
            entered_at: entered(),
            entered_by: KEYER.to_owned(),
        })
    );
    assert_eq!(
        desk.store
            .backdate_mark_in(&mut tx, TransactionId(Uuid::new_v4()))
            .await
            .unwrap(),
        None,
        "no such exchange"
    );
    tx.rollback().await.unwrap();
    desk.db.finish().await;
}

/// S3: a backdated draft naming a shift, or a DEMO one, is refused before any write.
#[actix_web::test]
async fn s3_a_shift_or_demo_draft_is_refused_before_any_write() {
    let desk = start().await;
    let o = org(&desk).await;
    let mut with_shift = draft(o);
    with_shift.shift_id = Some("SH-ANY".to_owned());
    let mut demo = draft(o);
    demo.mode = Mode::Demo;
    demo.customer_id = CUSTOMER.to_owned();
    for (what, t) in [("a shift", with_shift), ("DEMO", demo)] {
        let mut tx = desk.db.pool.begin().await.unwrap();
        let got = desk.store.insert_backdated_draft_in(&mut tx, &t).await;
        assert!(
            code(got).starts_with("validationFailed"),
            "{what} is refused validationFailed"
        );
        tx.rollback().await.unwrap();
    }
    assert_eq!(desk.db.count("SELECT count(*) FROM transactions").await, 0);
    desk.db.finish().await;
}

/// S4 (B13): the ordinary insert is unchanged: a LIVE draft with no shift is still refused by the schema.
#[actix_web::test]
async fn s4_the_ordinary_insert_still_refuses_a_live_draft_with_no_shift() {
    let desk = start().await;
    let t = draft(org(&desk).await);
    let mut tx = desk.db.pool.begin().await.unwrap();
    let got = desk.store.insert_draft_in(&mut tx, &t).await;
    assert!(got.is_err(), "transactions_shift_by_mode_check refuses it");
    tx.rollback().await.unwrap();
    desk.db.finish().await;
}

/// S5 (owner, design §11 Q1 = B): a backdated draft with no passport scan writes no PASSPORT row and reads back
/// with an empty `passport_file_key`.
#[actix_web::test]
async fn s5_a_backdated_draft_without_a_passport_writes_no_passport_row() {
    let desk = start().await;
    let mut t = draft(org(&desk).await);
    t.passport_file_key = String::new();
    let mut tx = desk.db.pool.begin().await.unwrap();
    desk.store
        .insert_backdated_draft_in(&mut tx, &t)
        .await
        .expect("a backdated draft needs no passport");
    let read = desk.store.find_in(&mut tx, t.id).await.unwrap().expect("it reads back");
    assert_eq!(read.passport_file_key, "");
    tx.commit().await.unwrap();
    assert_eq!(
        desk.db
            .count(&format!(
                "SELECT count(*) FROM transaction_files WHERE transaction_id = '{}'",
                t.id.0
            ))
            .await,
        0
    );
    desk.db.finish().await;
}
