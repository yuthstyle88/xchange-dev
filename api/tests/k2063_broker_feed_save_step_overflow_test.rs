//! K2063 (#2858): BrokerFeedSaveStep money arithmetic overflow hardening.
//!
//! Replaces unchecked `saved.principal.minor()` with checked `try_minor()`,
//! returning `AppError::Internal` when principal exceeds `i64::MAX` satang.

mod compat_common;
mod snapshot_common;

use std::str::FromStr;

use chrono::{NaiveDate, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use snapshot_common::TestDb;
use uuid::Uuid;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_customer::domain::customer::value_objects::customer_details::CustomerDetails;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{Money, Quote, Rate, Side};
use xchange::application::workflows::broker_feed_save_step::BrokerFeedSaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveContext, SaveExtras, SaveStep};

fn dummy_form(_branch: &BranchId, customer_id: &str) -> TransactionForm {
    TransactionForm {
        customer: CustomerDetails {
            name: "Test Customer".to_string(),
            passport_no: "P1234567".to_string(),
            nationality: "THA".to_string(),
            birth_date: Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap()),
            passport_expiry: Some(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap()),
            mode: Mode::Live,
        },
        submission_id: Uuid::new_v4().to_string(),
        customer_id: customer_id.to_string(),
        side: Side::Buy,
        currency: "USD".to_string(),
        amount: Money::parse("100").unwrap(),
        rate: Rate::parse("32.2").unwrap(),
        client_rate: Decimal::from_str("32.2").unwrap(),
        fee: Money::ZERO,
        bank_fee: Money::ZERO,
        method: PaymentMethod::Cash,
        payment_ref: String::new(),
        account: "Cash Drawer".to_string(),
        staff: "teller-1".to_string(),
        booking: "BK-test".to_string(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
        paid_at: Utc::now(),
        passport: String::new(),
        slip: None,
        reference_rate: None,
        client_reference_rate: None,
        quote: Quote {
            principal: Money::ZERO,
            received: Money::ZERO,
            paid: Money::ZERO,
        },
    }
}

fn dummy_saved(doc_ref: &str, principal: Money) -> Transaction {
    Transaction {
        document_ref: doc_ref.to_string(),
        branch_id: BranchId::new("MAIN"),
        mode: Mode::Live,
        shift_id: Some("SHIFT-1".to_string()),
        submission_id: format!("sub-{doc_ref}"),
        customer_id: "CUST-001".to_string(),
        customer_name: "Customer".to_string(),
        nationality: "THA".to_string(),
        side: Side::Sell,
        currency: "USD".to_string(),
        amount: Money::from_minor(100).unwrap(),
        rate: Rate::parse("35.0").unwrap(),
        reference_rate: None,
        principal,
        fee: Money::ZERO,
        bank_fee: Money::ZERO,
        received: Money::from_minor(100).unwrap(),
        paid: Money::ZERO,
        method: PaymentMethod::Cash,
        payment_ref: String::new(),
        account: String::new(),
        staff: "owner".to_string(),
        booking: String::new(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        paid_at: Utc::now(),
        passport_key: String::new(),
        slip_key: None,
        receipt_key: String::new(),
        recorded_at: Utc::now(),
        recorded_by: "owner".to_string(),
    }
}

async fn seed_fixtures(db: &TestDb, doc_ref: &str) -> Uuid {
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CUST-001', b.organization_id, 'CUST-001', 'Customer', 'LIVE', now() \
         FROM branches b WHERE b.id = 'MAIN' \
         ON CONFLICT DO NOTHING",
    )
    .execute(&db.pool)
    .await
    .expect("seed customer");

    let tx_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2, 'MAIN', 'DEMO', NULL, $3, 'payload-hash', 'CUST-001', 'Customer', 'THA', \
             'owner', '', '', \
             DATE '2026-10-06', now(), now(), 'owner', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(doc_ref)
    .bind(format!("sub-{doc_ref}"))
    .execute(&db.pool)
    .await
    .expect("seed transaction");

    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
             quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
         SELECT gen_random_uuid(), b.organization_id, $1, 'SELL', 'USD', \
             100, 1, 35, 3500, 0, 3500, 1, now(), now() \
         FROM branches b WHERE b.id = 'MAIN'",
    )
    .bind(tx_id)
    .execute(&db.pool)
    .await
    .expect("seed transaction line");

    tx_id
}

#[tokio::test]
async fn overflowing_principal_is_refused_with_app_error_internal() {
    let db = TestDb::start().await;
    let doc_ref = "DOC-OVERFLOW";
    let tx_id = seed_fixtures(&db, doc_ref).await;

    let branch = BranchId::new("MAIN");
    let form = dummy_form(&branch, "CUST-001");
    let extras = SaveExtras::default();
    let actor = SaveActor {
        id: "teller-1".to_string(),
        username: "teller-1".to_string(),
        employee_id: Some("teller-1".to_string()),
        standing: StandingKind::Staff(AccessRole::Teller),
    };
    let ctx = SaveContext {
        branch: &branch,
        form: &form,
        extras: &extras,
        booking: None,
        actor: &actor,
        teller: None,
        now: Utc::now(),
        prior: None,
    };

    // (1 << 64) + 5000 wraps to 5000 with unchecked `minor()` (as i64),
    // but fails with checked `try_minor()`.
    let overflow_minor = (1_i128 << 64) + 5000;
    let overflow_principal =
        Money::try_from_minor_i128(overflow_minor).expect("fits within Decimal minor range");
    let saved = dummy_saved(doc_ref, overflow_principal);

    let step = BrokerFeedSaveStep::new();
    let mut tx = db.pool.begin().await.expect("begin db transaction");

    let result = step.after_in(&mut tx, &ctx, &saved).await;

    match result {
        Err(AppError::Internal(msg)) => {
            assert!(
                msg.contains("principal overflow"),
                "expected 'principal overflow' in error message, got: {msg}"
            );
        }
        other => {
            panic!("expected Err(AppError::Internal(principal overflow)), got {other:?}");
        }
    }

    tx.rollback().await.expect("rollback");

    let feed_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM broker_exchange_feed WHERE transaction_id = $1")
            .bind(tx_id)
            .fetch_one(&db.pool)
            .await
            .expect("query feed count");

    assert_eq!(feed_count, 0, "no feed row should be written on overflow");
}

#[tokio::test]
async fn ordinary_principal_records_completed_broker_exchange_feed() {
    let db = TestDb::start().await;
    let doc_ref = "DOC-NORMAL";
    let tx_id = seed_fixtures(&db, doc_ref).await;

    let branch = BranchId::new("MAIN");
    let form = dummy_form(&branch, "CUST-001");
    let extras = SaveExtras::default();
    let actor = SaveActor {
        id: "teller-1".to_string(),
        username: "teller-1".to_string(),
        employee_id: Some("teller-1".to_string()),
        standing: StandingKind::Staff(AccessRole::Teller),
    };
    let ctx = SaveContext {
        branch: &branch,
        form: &form,
        extras: &extras,
        booking: None,
        actor: &actor,
        teller: None,
        now: Utc::now(),
        prior: None,
    };

    let normal_principal = Money::from_minor(5000).unwrap();
    let saved = dummy_saved(doc_ref, normal_principal);

    let step = BrokerFeedSaveStep::new();
    let mut tx = db.pool.begin().await.expect("begin db transaction");

    step.after_in(&mut tx, &ctx, &saved)
        .await
        .expect("after_in should succeed for normal principal");

    tx.commit().await.expect("commit");

    let feed_value: i64 = sqlx::query_scalar(
        "SELECT value_satang FROM broker_exchange_feed WHERE transaction_id = $1",
    )
    .bind(tx_id)
    .fetch_one(&db.pool)
    .await
    .expect("query feed value");

    assert_eq!(feed_value, 5000, "feed value should match principal satang");
}
