//! Ticket #1137 (TX-8m) — `PgSavedExchanges` answers from the exchange's status:
//! - a live COMPLETED exchange → `Some(ExchangeState::Completed)`;
//! - a DEMO completed exchange, an unknown number, or a reference that is not a COMPLETED exchange's number → `None`.
//!
//! (Option A ruling: under TX-2, an unfinished exchange has no document number, and devices act on printed numbers.
//! `NotCompleted` stays unanswered for now and keeps no producer until a later design gives unfinished exchanges a reference.)
//!
//! Written before the implementation (rule A, test-writer). Do not edit, delete or weaken.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod quote_common;
mod save_common;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use save_common::*;
use uuid::Uuid;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xchange::infrastructure::devices::pg_saved_exchanges::PgSavedExchanges;
use xchange::infrastructure::devices::saved_exchanges::{ExchangeState, SavedExchanges};

fn exchanges(shop: &Shop) -> PgSavedExchanges {
    PgSavedExchanges::new(shop.transactions.clone())
}

async fn find(of: &PgSavedExchanges, branch_id: &str, reference: &str) -> Option<ExchangeState> {
    of.find(&branch(branch_id), reference)
        .await
        .unwrap_or_else(|e| panic!("find {branch_id}/{reference}: {e:?}"))
}

fn sample_draft(
    organization_id: Uuid,
    branch: &str,
    customer_id: &str,
    passport_file_key: &str,
    shift_id: Option<String>,
    mode: Mode,
    now: DateTime<Utc>,
) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    ExchangeTransaction {
        id,
        organization_id,
        branch_id: BranchId::new(branch.to_owned()),
        mode,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id,
        submission_id: format!("sub-{}", id.0),
        payload_hash: "hash".to_owned(),
        customer_id: customer_id.to_owned(),
        customer_name: "Somchai".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "EMP-001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: "a note".to_owned(),
        value_date: now.date_naive(),
        recorded_at: now,
        recorded_by: "owner".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: passport_file_key.to_owned(),
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
            thb_amount: Money::parse("3210.00").unwrap(),
            fee_thb: Money::parse("10.00").unwrap(),
            settle_thb: Money::parse("3200.00").unwrap(),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::BankTransfer,
            reference: "TRX-9".to_owned(),
            account: "KBank 1".to_owned(),
            slip_file_key: None,
            amount_thb: Money::parse("3200.00").unwrap(),
            bank_fee_thb: Money::parse("1.50").unwrap(),
            paid_at: now,
        },
        receipts: vec![],
    }
}

#[tokio::test]
async fn a_live_completed_exchange_gives_completed() {
    let shop = Shop::start().await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .unwrap();

    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    let receipt_key = "RECEIPT-MAIN";
    sqlx::query("INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES ($1, 'MAIN', 'owner', 'application/pdf', 'sha-rcpt', now()) ON CONFLICT (key) DO NOTHING")
        .bind(receipt_key)
        .execute(&shop.pool)
        .await
        .unwrap();

    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = sample_draft(
        org_id,
        MAIN,
        &customer,
        &passport,
        Some(shift_id),
        Mode::Live,
        at(0),
    );
    let id = draft.id;
    let mut tx = shop.pool.begin().await.unwrap();
    store.insert_draft_in(&mut tx, &draft).await.unwrap();
    store
        .set_status_in(
            &mut tx,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(10),
        )
        .await
        .unwrap();
    let (doc_ref, _) = store
        .complete_in(&mut tx, id, receipt_key, at(20))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'RECEIPT', $2)",
    )
    .bind(id.0)
    .bind(receipt_key)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let of = exchanges(&shop);
    assert_eq!(
        find(&of, MAIN, &doc_ref).await,
        Some(ExchangeState::Completed)
    );
}

#[tokio::test]
async fn a_compat_saved_live_exchange_gives_completed() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let live = shop
        .save(
            MAIN,
            form("live-0001-compat-test", &customer, &passport),
            at(0),
        )
        .await
        .expect("the live save");
    assert_eq!(live.mode, Mode::Live);

    let of = exchanges(&shop);
    assert_eq!(
        find(&of, MAIN, &live.document_ref).await,
        Some(ExchangeState::Completed)
    );
}

#[tokio::test]
async fn a_live_draft_exchange_gives_none() {
    let shop = Shop::start().await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .unwrap();

    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = sample_draft(
        org_id,
        MAIN,
        &customer,
        &passport,
        Some(shift_id),
        Mode::Live,
        at(0),
    );
    let mut tx = shop.pool.begin().await.unwrap();
    store.insert_draft_in(&mut tx, &draft).await.unwrap();
    tx.commit().await.unwrap();

    let of = exchanges(&shop);
    // Under TX-2, an unfinished exchange has no document number (NULL).
    // Devices act on printed numbers: a reference that is not a completed exchange's number answers None.
    assert_eq!(find(&of, MAIN, "XC-DRAFT-000001").await, None);
}

#[tokio::test]
async fn a_live_confirmed_exchange_gives_none() {
    let shop = Shop::start().await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .unwrap();

    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = sample_draft(
        org_id,
        MAIN,
        &customer,
        &passport,
        Some(shift_id),
        Mode::Live,
        at(0),
    );
    let id = draft.id;
    let mut tx = shop.pool.begin().await.unwrap();
    store.insert_draft_in(&mut tx, &draft).await.unwrap();
    store
        .set_status_in(
            &mut tx,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(10),
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let of = exchanges(&shop);
    assert_eq!(find(&of, MAIN, "XC-CONFIRMED-000001").await, None);
}

#[tokio::test]
async fn a_live_cancelled_exchange_gives_none() {
    let shop = Shop::start().await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .unwrap();

    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = sample_draft(
        org_id,
        MAIN,
        &customer,
        &passport,
        Some(shift_id),
        Mode::Live,
        at(0),
    );
    let id = draft.id;
    let mut tx = shop.pool.begin().await.unwrap();
    store.insert_draft_in(&mut tx, &draft).await.unwrap();
    store
        .set_status_in(
            &mut tx,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Cancelled,
                reason: Some("customer cancelled".to_owned()),
                approval_request_id: None,
            },
            at(10),
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let of = exchanges(&shop);
    assert_eq!(find(&of, MAIN, "XC-CANCELLED-000001").await, None);
}

#[tokio::test]
async fn a_demo_completed_exchange_gives_none() {
    let shop = Shop::start().await;
    let (_customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let mut demo_person = person();
    demo_person.mode = "DEMO".to_owned();
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let mut demo = form("demo-0001-compat-test", &demo_customer, &passport);
    demo.customer = demo_person;
    let demo = shop.save(MAIN, demo, at(60)).await.expect("the DEMO save");
    assert_eq!(demo.mode, Mode::Demo);

    let of = exchanges(&shop);
    assert_eq!(find(&of, MAIN, &demo.document_ref).await, None);
}

#[tokio::test]
async fn an_unknown_number_gives_none() {
    let shop = Shop::start().await;
    let of = exchanges(&shop);
    assert_eq!(find(&of, MAIN, "XC-20990101-000001").await, None);
    assert_eq!(find(&of, MAIN, "").await, None);
}

#[tokio::test]
async fn an_unfinished_exchange_with_a_document_number_gives_none() {
    let shop = Shop::start().await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .unwrap();

    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    let receipt_key = "RECEIPT-MAIN";
    sqlx::query("INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES ($1, 'MAIN', 'owner', 'application/pdf', 'sha-rcpt', now()) ON CONFLICT (key) DO NOTHING")
        .bind(receipt_key)
        .execute(&shop.pool)
        .await
        .unwrap();

    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = sample_draft(
        org_id,
        MAIN,
        &customer,
        &passport,
        Some(shift_id),
        Mode::Live,
        at(0),
    );
    let id = draft.id;
    let mut tx = shop.pool.begin().await.unwrap();
    store.insert_draft_in(&mut tx, &draft).await.unwrap();
    store
        .set_status_in(
            &mut tx,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(10),
        )
        .await
        .unwrap();
    let (doc_ref, _) = store
        .complete_in(&mut tx, id, receipt_key, at(20))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'RECEIPT', $2)",
    )
    .bind(id.0)
    .bind(receipt_key)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Drop the check constraint so we can test that the reader checks status = 'COMPLETED'
    // even if a non-completed row has a document_ref (design §4.1, #1137 §3, #1138).
    sqlx::query(
        "ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_completed_document_ref_check",
    )
    .execute(&shop.pool)
    .await
    .unwrap();

    sqlx::query("UPDATE transactions SET status = 'CONFIRMED', completed_at = NULL WHERE id = $1")
        .bind(id.0)
        .execute(&shop.pool)
        .await
        .unwrap();

    let of = exchanges(&shop);
    assert_eq!(
        find(&of, MAIN, &doc_ref).await,
        None,
        "an unfinished (CONFIRMED) exchange with a document number must answer None"
    );
}

#[tokio::test]
async fn a_cancelled_exchange_with_a_document_number_gives_none() {
    let shop = Shop::start().await;
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
        .fetch_one(&shop.pool)
        .await
        .unwrap();

    let (customer, passport) = shop.ready(&[("USD", "1000"), ("THB", "100000")]).await;
    let shift_id: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(&shop.pool)
            .await
            .unwrap();
    let receipt_key = "RECEIPT-MAIN";
    sqlx::query("INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES ($1, 'MAIN', 'owner', 'application/pdf', 'sha-rcpt', now()) ON CONFLICT (key) DO NOTHING")
        .bind(receipt_key)
        .execute(&shop.pool)
        .await
        .unwrap();

    let store = PgTransactionStore::new(shop.pool.clone());
    let draft = sample_draft(
        org_id,
        MAIN,
        &customer,
        &passport,
        Some(shift_id),
        Mode::Live,
        at(0),
    );
    let id = draft.id;
    let mut tx = shop.pool.begin().await.unwrap();
    store.insert_draft_in(&mut tx, &draft).await.unwrap();
    store
        .set_status_in(
            &mut tx,
            id,
            StatusMove {
                from: TransactionStatus::Draft,
                to: TransactionStatus::Confirmed,
                reason: None,
                approval_request_id: None,
            },
            at(10),
        )
        .await
        .unwrap();
    let (doc_ref, _) = store
        .complete_in(&mut tx, id, receipt_key, at(20))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'RECEIPT', $2)",
    )
    .bind(id.0)
    .bind(receipt_key)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Drop the check constraint so we can test that the reader checks status = 'COMPLETED'
    // even if a non-completed row has a document_ref (design §4.1, #1137 §3, #1138).
    sqlx::query(
        "ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_completed_document_ref_check",
    )
    .execute(&shop.pool)
    .await
    .unwrap();

    sqlx::query(
        "UPDATE transactions SET status = 'CANCELLED', completed_at = NULL, cancelled_at = now(), cancel_reason = 'test' WHERE id = $1",
    )
    .bind(id.0)
    .execute(&shop.pool)
    .await
    .unwrap();

    let of = exchanges(&shop);
    assert_eq!(
        find(&of, MAIN, &doc_ref).await,
        None,
        "a CANCELLED exchange with a document number must answer None"
    );
}
