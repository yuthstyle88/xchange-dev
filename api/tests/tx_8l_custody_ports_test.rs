//! Ticket #1136 (TX-8l) — the custody check's `outside_facts`:
//! - `last_transaction`: the shift's latest `COMPLETED` exchange's `document_ref` (never a non-COMPLETED row);
//! - `pending_transactions`: the ids of the shift's unfinished exchanges (`DRAFT`, `PENDING_APPROVAL`, `CONFIRMED`), oldest first.
//!
//! Written before the implementation by the test writer (gemini/pool-2, rule A).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod save_common;

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use save_common::*;
use uuid::Uuid;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Rate, Side, UnitSize};
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

fn ports() -> Arc<dyn CustodyPorts> {
    Arc::new(PgCustodyPorts)
}

async fn outside(shop: &Shop, branch_id: &str, shift: Option<&str>) -> OutsideFacts {
    let mut tx = shop.pool.begin().await.expect("begin");
    let facts = ports()
        .outside_facts(&mut tx, &branch(branch_id), shift)
        .await
        .expect("outside_facts");
    tx.rollback().await.expect("rollback");
    facts
}

async fn org_id(shop: &Shop) -> Uuid {
    sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .expect("branch organization_id")
}

fn make_draft(
    organization_id: Uuid,
    branch_id: &str,
    shift_id: &str,
    customer_id: &str,
    passport_key: &str,
    seq_num: u32,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    ExchangeTransaction {
        id,
        organization_id,
        branch_id: branch(branch_id),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift_id.to_owned()),
        submission_id: format!("sub-{}-{seq_num}", id.0),
        payload_hash: format!("hash-{seq_num}"),
        customer_id: customer_id.to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "EMP-001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "test note".to_owned(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        recorded_at: at(seq_num as i64),
        recorded_by: OWNER.to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: passport_key.to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from(100)),
            unit_size: UnitSize::parse("1").unwrap(),
            rate_snapshot: Rate::parse("32.1").unwrap(),
            reference_rate: Some(Rate::parse("32.05").unwrap()),
            thb_amount: money("3210.00"),
            fee_thb: money("10.00"),
            settle_thb: money("3200.00"),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::Cash,
            reference: format!("PAY-{seq_num}"),
            account: "Cash drawer 1".to_owned(),
            slip_file_key: None,
            amount_thb: money("3200.00"),
            bank_fee_thb: money("0.00"),
            paid_at: at(seq_num as i64),
        },
        receipts: vec![],
    }
}

async fn seed_draft(shop: &Shop, store: &PgTransactionStore, t: &ExchangeTransaction) {
    let mut tx = shop.pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut tx, t)
        .await
        .expect("insert_draft");
    tx.commit().await.expect("commit");
}

async fn seed_confirmed(
    shop: &Shop,
    store: &PgTransactionStore,
    t: &ExchangeTransaction,
    now: DateTime<Utc>,
) {
    let mut tx = shop.pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut tx, t)
        .await
        .expect("insert_draft");
    store
        .set_status_in(
            &mut tx,
            t.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            now,
        )
        .await
        .expect("set_status to Confirmed");
    tx.commit().await.expect("commit");
}

async fn seed_pending_approval(
    shop: &Shop,
    store: &PgTransactionStore,
    t: &ExchangeTransaction,
    now: DateTime<Utc>,
) {
    let mut tx = shop.pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut tx, t)
        .await
        .expect("insert_draft");
    store
        .set_status_in(
            &mut tx,
            t.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::PendingApproval,
                reason: None,
                approval_request_id: None,
            },
            now,
        )
        .await
        .expect("set_status to PendingApproval");
    tx.commit().await.expect("commit");
}

async fn seed_cancelled(
    shop: &Shop,
    store: &PgTransactionStore,
    t: &ExchangeTransaction,
    now: DateTime<Utc>,
    reason: &str,
) {
    let mut tx = shop.pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut tx, t)
        .await
        .expect("insert_draft");
    store
        .set_status_in(
            &mut tx,
            t.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Cancelled,
                reason: Some(reason.to_owned()),
                approval_request_id: None,
            },
            now,
        )
        .await
        .expect("set_status to Cancelled");
    tx.commit().await.expect("commit");
}

async fn seed_completed(
    shop: &Shop,
    store: &PgTransactionStore,
    t: &ExchangeTransaction,
    receipt_key: &str,
    now: DateTime<Utc>,
) -> String {
    let mut tx = shop.pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut tx, t)
        .await
        .expect("insert_draft");
    store
        .set_status_in(
            &mut tx,
            t.id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            now,
        )
        .await
        .expect("set_status to Confirmed");
    let (doc_ref, _receipt) = store
        .complete_in(&mut tx, t.id, receipt_key, now)
        .await
        .expect("complete_in");
    tx.commit().await.expect("commit");
    doc_ref
}

// ---------------------------------------------------------------- Edge cases

/// When an unfinished exchange precedes a completed exchange, `last_transaction`
/// reports the completed exchange, and `pending_transactions` contains the unfinished one.
/// On the base branch (which returns `pending_transactions: Vec::new()`), this fails on a wrong value.
#[tokio::test]
async fn shift_with_unfinished_before_completed_reports_completed_as_last_and_unfinished_as_pending()
 {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let org = org_id(&shop).await;
    let receipt = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let store = PgTransactionStore::new(shop.pool.clone());

    // 1. One DRAFT exchange
    let t_draft = make_draft(org, MAIN, &shift, &customer, &passport, 1);
    seed_draft(&shop, &store, &t_draft).await;

    // 2. One COMPLETED exchange (latest in shift)
    let t_completed = make_draft(org, MAIN, &shift, &customer, &passport, 2);
    let completed_ref = seed_completed(&shop, &store, &t_completed, &receipt, at(2)).await;

    let facts = outside(&shop, MAIN, Some(&shift)).await;

    assert_eq!(
        facts.last_transaction,
        Some(completed_ref),
        "last_transaction must be the completed exchange"
    );
    assert_eq!(
        facts.pending_transactions,
        vec![t_draft.id.0.to_string()],
        "pending_transactions must contain the DRAFT id"
    );

    shop.finish().await;
}

/// §9 edge case 1: A shift with the four seeded rows:
/// `last_transaction` = the completed one's number;
/// `pending_transactions` = the DRAFT and CONFIRMED ids, oldest first.
/// CANCELLED is neither completed nor unfinished, so it must not be in either.
#[tokio::test]
async fn shift_with_the_four_seeded_rows_reports_completed_number_and_unfinished_ids_oldest_first()
{
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let org = org_id(&shop).await;
    let receipt = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let store = PgTransactionStore::new(shop.pool.clone());

    // 1. One COMPLETED exchange
    let t_completed = make_draft(org, MAIN, &shift, &customer, &passport, 1);
    let completed_ref = seed_completed(&shop, &store, &t_completed, &receipt, at(1)).await;

    // 2. One DRAFT exchange
    let t_draft = make_draft(org, MAIN, &shift, &customer, &passport, 2);
    seed_draft(&shop, &store, &t_draft).await;

    // 3. One CONFIRMED exchange
    let t_confirmed = make_draft(org, MAIN, &shift, &customer, &passport, 3);
    seed_confirmed(&shop, &store, &t_confirmed, at(3)).await;

    // 4. One CANCELLED exchange
    let t_cancelled = make_draft(org, MAIN, &shift, &customer, &passport, 4);
    seed_cancelled(&shop, &store, &t_cancelled, at(4), "customer cancel").await;

    let facts = outside(&shop, MAIN, Some(&shift)).await;

    assert_eq!(
        facts.last_transaction,
        Some(completed_ref),
        "last_transaction must be the latest COMPLETED exchange's document_ref"
    );
    assert_eq!(
        facts.pending_transactions,
        vec![t_draft.id.0.to_string(), t_confirmed.id.0.to_string()],
        "pending_transactions must contain the DRAFT and CONFIRMED ids, oldest first"
    );

    shop.finish().await;
}

/// Unfinished is `status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED')` (§4.1):
/// PENDING_APPROVAL must also be reported in `pending_transactions`, oldest first.
#[tokio::test]
async fn shift_with_pending_approval_reports_it_in_pending_transactions_oldest_first() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let org = org_id(&shop).await;
    let store = PgTransactionStore::new(shop.pool.clone());

    let t1_draft = make_draft(org, MAIN, &shift, &customer, &passport, 1);
    seed_draft(&shop, &store, &t1_draft).await;

    let t2_pending = make_draft(org, MAIN, &shift, &customer, &passport, 2);
    seed_pending_approval(&shop, &store, &t2_pending, at(2)).await;

    let t3_confirmed = make_draft(org, MAIN, &shift, &customer, &passport, 3);
    seed_confirmed(&shop, &store, &t3_confirmed, at(3)).await;

    let t4_cancelled = make_draft(org, MAIN, &shift, &customer, &passport, 4);
    seed_cancelled(&shop, &store, &t4_cancelled, at(4), "rejected").await;

    let facts = outside(&shop, MAIN, Some(&shift)).await;

    assert_eq!(
        facts.last_transaction, None,
        "no completed exchange in shift"
    );
    assert_eq!(
        facts.pending_transactions,
        vec![
            t1_draft.id.0.to_string(),
            t2_pending.id.0.to_string(),
            t3_confirmed.id.0.to_string(),
        ],
        "pending_transactions must include DRAFT, PENDING_APPROVAL, and CONFIRMED in oldest-first order"
    );

    shop.finish().await;
}

/// Ticket description: "plus one compat-saved exchange (the wide and child figures equal)".
/// A compat save produces a COMPLETED exchange, so it is reported as `last_transaction`
/// and leaves `pending_transactions` empty.
#[tokio::test]
async fn compat_saved_exchange_is_completed_and_reported_as_last_transaction_with_no_pending() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;

    let saved = shop
        .save(
            MAIN,
            form("compat-0001-aaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .expect("compat save");

    let facts = outside(&shop, MAIN, Some(&shift)).await;

    assert_eq!(
        facts.last_transaction,
        Some(saved.document_ref),
        "compat-saved exchange is COMPLETED and must be reported as last_transaction"
    );
    assert!(
        facts.pending_transactions.is_empty(),
        "compat-saved exchange has no unfinished exchanges"
    );

    shop.finish().await;
}

/// §9 edge case 2: A shift with none: `None` and empty, as today.
#[tokio::test]
async fn shift_with_none_reports_none_and_empty() {
    let shop = Shop::start().await;
    let (_customer, _passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;

    let facts = outside(&shop, MAIN, Some(&shift)).await;

    assert_eq!(facts.last_transaction, None);
    assert!(facts.pending_transactions.is_empty());

    // Without a shift parameter, also None and empty
    let no_shift = outside(&shop, MAIN, None).await;
    assert_eq!(no_shift.last_transaction, None);
    assert!(no_shift.pending_transactions.is_empty());

    shop.finish().await;
}

/// When an unfinished (DRAFT) or CANCELLED exchange is recorded after a COMPLETED exchange,
/// `last_transaction` must STILL report the latest COMPLETED exchange's document_ref,
/// not NULL or None.
#[tokio::test]
async fn unfinished_or_cancelled_exchange_after_completed_does_not_mask_last_completed_transaction()
{
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let org = org_id(&shop).await;
    let receipt = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let store = PgTransactionStore::new(shop.pool.clone());

    // 1. First exchange is completed
    let t_completed = make_draft(org, MAIN, &shift, &customer, &passport, 1);
    let completed_ref = seed_completed(&shop, &store, &t_completed, &receipt, at(1)).await;

    // 2. Later exchange is a DRAFT (document_ref is NULL)
    let t_draft = make_draft(org, MAIN, &shift, &customer, &passport, 2);
    seed_draft(&shop, &store, &t_draft).await;

    // 3. Even later exchange is CANCELLED (document_ref is NULL)
    let t_cancelled = make_draft(org, MAIN, &shift, &customer, &passport, 3);
    seed_cancelled(&shop, &store, &t_cancelled, at(3), "cancel").await;

    let facts = outside(&shop, MAIN, Some(&shift)).await;

    assert_eq!(
        facts.last_transaction,
        Some(completed_ref),
        "last_transaction must be the latest COMPLETED exchange, not masked by later unfinished/cancelled ones"
    );
    assert_eq!(
        facts.pending_transactions,
        vec![t_draft.id.0.to_string()],
        "pending_transactions must list the later DRAFT exchange"
    );

    shop.finish().await;
}

/// Unfinished exchanges in another shift or branch are isolated and never reported.
#[tokio::test]
async fn unfinished_exchanges_in_another_branch_or_shift_are_not_reported() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift_1 = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let org = org_id(&shop).await;
    let store = PgTransactionStore::new(shop.pool.clone());

    // In shift 1 of MAIN: one DRAFT
    let t1 = make_draft(org, MAIN, &shift_1, &customer, &passport, 1);
    seed_draft(&shop, &store, &t1).await;

    // Close shift 1, open shift 2 on MAIN
    shop.close_shift(MAIN).await;
    let shift_2 = shop
        .open_shift(MAIN, &[("THB", "100000"), ("USD", "1000")])
        .await;

    // In shift 2 of MAIN: another DRAFT
    let t2 = make_draft(org, MAIN, &shift_2, &customer, &passport, 2);
    seed_draft(&shop, &store, &t2).await;

    // Shift 1 must see only t1
    let facts_1 = outside(&shop, MAIN, Some(&shift_1)).await;
    assert_eq!(
        facts_1.pending_transactions,
        vec![t1.id.0.to_string()],
        "shift_1 must only see its own pending transaction"
    );

    // Shift 2 must see only t2
    let facts_2 = outside(&shop, MAIN, Some(&shift_2)).await;
    assert_eq!(
        facts_2.pending_transactions,
        vec![t2.id.0.to_string()],
        "shift_2 must only see its own pending transaction"
    );

    // Another branch (B01)
    shop.seed_branch("XC-B01", "Second Branch").await;
    let facts_b01 = outside(&shop, "XC-B01", Some(&shift_1)).await;
    assert!(facts_b01.pending_transactions.is_empty());
    assert_eq!(facts_b01.last_transaction, None);

    shop.finish().await;
}
