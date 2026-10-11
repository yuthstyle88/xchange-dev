//! Ticket #1138 (TX-8n) — `DeviceService::after_save_in`: reads the sum of the line `thb_amount`
//! of the COMPLETED exchange with that document number in the device's branch.
//!
//! Written before the implementation by the test-writer (rule A). Do not edit, delete or weaken.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` (#551): §3.2–§3.4, §4.1, §9 row TX-8.
//! - A reader never counts a non-COMPLETED row as a sale: `status = 'COMPLETED'`.
//! - A reader never reads a figure the line holds from the wide row: `principal` is read instead
//!   from `transaction_lines.thb_amount` (summed per transaction).
//! - The rewritten read filters by branch (`AND t.branch_id = $2`) because after TX-2 D-4,
//!   `document_ref` is unique only per organisation.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::infrastructure::devices::device_commands::SavedExchange;
use xchange::infrastructure::devices::device_service::DeviceService;
use xchange::infrastructure::devices::job_ids::UuidJobIds;
use xchange::infrastructure::devices::saved_exchanges::{ExchangeState, SavedExchanges};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const ORG_2: &str = "00000000-0000-0000-0000-0000000000b2";

struct NoExchanges;

impl SavedExchanges for NoExchanges {
    fn find<'a>(
        &'a self,
        _branch: &'a BranchId,
        _reference: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<ExchangeState>>> {
        Box::pin(async move { Ok(None) })
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

fn branch(name: &str) -> BranchId {
    BranchId::new(name.to_owned())
}

fn build_service(db: &TestDb) -> DeviceService {
    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    DeviceService::new(branch_tx, Arc::new(NoExchanges), Arc::new(UuidJobIds))
}

async fn ensure_branch(db: &TestDb, branch_id: &str, org_id: &str) {
    if org_id == ORG_2 {
        sqlx::query(
            "CREATE OR REPLACE FUNCTION sync_device_org() RETURNS TRIGGER AS $$ \
             BEGIN \
                 IF NEW.branch_id IS NOT NULL THEN \
                     SELECT organization_id INTO NEW.organization_id FROM branches WHERE id = NEW.branch_id; \
                 END IF; \
                 RETURN NEW; \
             END; \
             $$ LANGUAGE plpgsql;",
        )
        .execute(&db.pool)
        .await
        .expect("create function sync_device_org");

        sqlx::query("DROP TRIGGER IF EXISTS device_events_sync_org ON device_events")
            .execute(&db.pool)
            .await
            .expect("drop trigger device_events_sync_org");

        sqlx::query(
            "CREATE TRIGGER device_events_sync_org BEFORE INSERT ON device_events \
             FOR EACH ROW EXECUTE FUNCTION sync_device_org()",
        )
        .execute(&db.pool)
        .await
        .expect("create trigger device_events_sync_org");

        sqlx::query("DROP TRIGGER IF EXISTS device_jobs_sync_org ON device_jobs")
            .execute(&db.pool)
            .await
            .expect("drop trigger device_jobs_sync_org");

        sqlx::query(
            "CREATE TRIGGER device_jobs_sync_org BEFORE INSERT ON device_jobs \
             FOR EACH ROW EXECUTE FUNCTION sync_device_org()",
        )
        .execute(&db.pool)
        .await
        .expect("create trigger device_jobs_sync_org");

        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1::uuid, 'ORG2', NULL, 'Asia/Bangkok', now()) ON CONFLICT (id) DO NOTHING",
        )
        .bind(org_id)
        .execute(&db.pool)
        .await
        .expect("seed org 2");
    }
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) \
         VALUES ($1, $1, $2::uuid) ON CONFLICT (id) DO NOTHING",
    )
    .bind(branch_id)
    .bind(org_id)
    .execute(&db.pool)
    .await
    .expect("seed branch");
}

#[allow(clippy::too_many_arguments)]
async fn seed_transaction(
    db: &TestDb,
    org_id: &str,
    branch_id: &str,
    document_ref: Option<&str>,
    submission_id: &str,
    status: &str,
    wide_principal: Decimal,
    lines: &[Decimal],
) {
    ensure_branch(db, branch_id, org_id).await;

    let cust_id = format!("cust-{branch_id}");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $1, 'Somchai', 'DEMO', now()) ON CONFLICT DO NOTHING",
    )
    .bind(&cust_id)
    .bind(org_id)
    .execute(&db.pool)
    .await
    .expect("seed customer");

    let tx_id = Uuid::new_v4();
    let completed_at = if status == "COMPLETED" {
        Some(Utc::now())
    } else {
        None
    };
    let (cancelled_at, cancel_reason) = if status == "CANCELLED" {
        (Some(Utc::now()), Some("customer cancelled"))
    } else {
        (None, None)
    };

    sqlx::query(
        "INSERT INTO transactions ( \
            id, organization_id, branch_id, mode, status, document_ref, \
            submission_id, payload_hash, customer_id, customer_name, nationality, \
            type, code, amount, rate, unit_size, principal, fee, bank_fee, \
            received, paid, method, payment_ref, account, staff_id, booking_no, \
            notes, value_date, paid_at, recorded_at, recorded_by, completed_at, \
            cancelled_at, cancel_reason \
        ) VALUES ( \
            $1, $2::uuid, $3, 'DEMO', $4, $5, \
            $6, 'hash', $7, 'Somchai', 'THA', \
            'BUY', 'USD', 100, 32, 1, $8, 0, 0, \
            0, $8, 'Cash', '', 'Cash drawer', 'owner', 'WALK-IN', \
            '', '2026-10-06', now(), now(), 'owner', $9, \
            $10, $11 \
        )",
    )
    .bind(tx_id)
    .bind(org_id)
    .bind(branch_id)
    .bind(status)
    .bind(document_ref)
    .bind(submission_id)
    .bind(&cust_id)
    .bind(wide_principal)
    .bind(completed_at)
    .bind(cancelled_at)
    .bind(cancel_reason)
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("seed transaction {document_ref:?}: {e}"));

    for (i, line_thb) in lines.iter().enumerate() {
        let line_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transaction_lines ( \
                id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, \
                settle_thb, line_no, created_at, updated_at \
            ) VALUES ( \
                $1, $2::uuid, $3, 'BUY', 'USD', \
                100, 1, 32, $4, 0, \
                $4, $5, now(), now() \
            )",
        )
        .bind(line_id)
        .bind(org_id)
        .bind(tx_id)
        .bind(line_thb)
        .bind(i as i32 + 1)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("seed transaction line: {e}"));
    }

    let payment_id = Uuid::new_v4();
    let total_thb: Decimal = lines.iter().sum();
    sqlx::query(
        "INSERT INTO transaction_payments ( \
            id, organization_id, transaction_id, payment_no, method, \
            amount_thb, bank_fee_thb, paid_at, created_at, updated_at \
        ) VALUES ( \
            $1, $2::uuid, $3, 1, 'Cash', \
            $4, 0, now(), now(), now() \
        )",
    )
    .bind(payment_id)
    .bind(org_id)
    .bind(tx_id)
    .bind(total_thb)
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("seed transaction payment: {e}"));

    if let Some(doc_ref) = document_ref {
        let receipt_file_key = format!("RECEIPT-{doc_ref}");
        let passport_file_key = format!("PASSPORT-{doc_ref}");

        sqlx::query(
            "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2::uuid, $3, 'owner', 'application/pdf', 'sha-rec', now()), \
                    ($4, $2::uuid, $3, 'owner', 'application/pdf', 'sha-pass', now()) \
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(&receipt_file_key)
        .bind(org_id)
        .bind(branch_id)
        .bind(&passport_file_key)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("seed files: {e}"));

        let receipt_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, document_ref, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 1, now(), $5, now(), now())",
        )
        .bind(receipt_id)
        .bind(org_id)
        .bind(tx_id)
        .bind(doc_ref)
        .bind(&receipt_file_key)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("seed receipt: {e}"));

        sqlx::query(
            "INSERT INTO transaction_files (transaction_id, kind, file_key) \
             VALUES ($1, 'RECEIPT', $2), ($1, 'PASSPORT', $3)",
        )
        .bind(tx_id)
        .bind(&receipt_file_key)
        .bind(&passport_file_key)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("seed transaction files: {e}"));
    }
}

// ──────────────────────────────── §9 Edge cases ─────────────────────────────

/// §9 edge case 3 (#1122): Two organisations, one number.
/// Organisation A's branch and organisation B's branch each have a completed exchange
/// numbered XC-20261010-000001, with different THB. The device of A's branch reads A's THB,
/// and B's device reads B's.
#[tokio::test]
async fn two_organisations_same_number_each_device_reads_its_own_branch_thb() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    let doc_ref = "XC-20261010-000001";
    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        Some(doc_ref),
        "sub-org1-001",
        "COMPLETED",
        d("1000.00"),
        &[d("1000.00")],
    )
    .await;

    seed_transaction(
        &db,
        ORG_2,
        "B2",
        Some(doc_ref),
        "sub-org2-001",
        "COMPLETED",
        d("2000.00"),
        &[d("2000.00")],
    )
    .await;

    // Org 1 branch MAIN
    let saved_main = SavedExchange {
        cash: true,
        submission_id: "sub-org1-001".to_owned(),
        document_ref: doc_ref.to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("10.00"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    service
        .after_save_in(&mut tx, &branch("MAIN"), &saved_main, "owner", Utc::now())
        .await
        .expect("after_save_in MAIN");
    tx.commit().await.unwrap();

    let thb_main: String = sqlx::query_scalar(
        "SELECT detail->>'thb' FROM device_events WHERE kind = 'COUNT_CONFIRMED' AND ref = $1",
    )
    .bind("sub-org1-001")
    .fetch_one(&db.pool)
    .await
    .expect("read main event");
    assert_eq!(d(&thb_main), d("1000.00"), "MAIN device reads Org 1's THB");

    // Org 2 branch B2
    let saved_b2 = SavedExchange {
        cash: true,
        submission_id: "sub-org2-001".to_owned(),
        document_ref: doc_ref.to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("20.00"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    service
        .after_save_in(&mut tx, &branch("B2"), &saved_b2, "owner", Utc::now())
        .await
        .expect("after_save_in B2");
    tx.commit().await.unwrap();

    let thb_b2: String = sqlx::query_scalar(
        "SELECT detail->>'thb' FROM device_events WHERE kind = 'COUNT_CONFIRMED' AND ref = $1",
    )
    .bind("sub-org2-001")
    .fetch_one(&db.pool)
    .await
    .expect("read B2 event");
    assert_eq!(d(&thb_b2), d("2000.00"), "B2 device reads Org 2's THB");

    db.finish().await;
}

/// §9 edge case 4: The same number at another branch of the same organisation is not
/// this branch's exchange: the same error as an unknown number.
#[tokio::test]
async fn same_number_at_another_branch_of_same_organisation_is_refused() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    let doc_ref = "XC-20261010-000002";
    // Seed exchange in branch MAIN
    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        Some(doc_ref),
        "sub-main-002",
        "COMPLETED",
        d("1234.00"),
        &[d("1234.00")],
    )
    .await;

    // Ensure branch B9 exists in ORG_1
    ensure_branch(&db, "B9", ORG_1).await;

    // Device at branch B9 attempts after_save_in with MAIN's document_ref
    let saved_b9 = SavedExchange {
        cash: true,
        submission_id: "sub-b9-002".to_owned(),
        document_ref: doc_ref.to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("12.34"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    let res = service
        .after_save_in(&mut tx, &branch("B9"), &saved_b9, "owner", Utc::now())
        .await;
    tx.rollback().await.unwrap();

    assert!(
        matches!(res, Err(AppError::Internal(_))),
        "same number at another branch of same org must be refused as unknown: {res:?}"
    );

    db.finish().await;
}

/// Done criterion 3 & §8: A reader never reads a figure the line holds from the wide row.
/// The device service reads the sum of the line `thb_amount` of the COMPLETED exchange,
/// not the wide row's `principal`.
#[tokio::test]
async fn completed_cash_exchange_reads_sum_of_lines_thb_not_wide_principal() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    let doc_ref = "XC-20261010-000003";
    // wide principal is 1000.00, but child lines sum to 2000.00 + 1500.00 = 3500.00
    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        Some(doc_ref),
        "sub-multiline-003",
        "COMPLETED",
        d("1000.00"),
        &[d("2000.00"), d("1500.00")],
    )
    .await;

    let saved = SavedExchange {
        cash: true,
        submission_id: "sub-multiline-003".to_owned(),
        document_ref: doc_ref.to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("35.00"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    service
        .after_save_in(&mut tx, &branch("MAIN"), &saved, "owner", Utc::now())
        .await
        .expect("after_save_in multiline");
    tx.commit().await.unwrap();

    let thb: String = sqlx::query_scalar(
        "SELECT detail->>'thb' FROM device_events WHERE kind = 'COUNT_CONFIRMED' AND ref = $1",
    )
    .bind("sub-multiline-003")
    .fetch_one(&db.pool)
    .await
    .expect("read event");

    assert_eq!(
        d(&thb),
        d("3500.00"),
        "device count line must read sum of child lines thb_amount (3500.00), not wide principal (1000.00)"
    );

    db.finish().await;
}

/// Design §4.1 & #1138 §3: A reader never counts a non-COMPLETED row as a sale:
/// `… WHERE t.document_ref = $1 AND t.branch_id = $2 AND t.status = 'COMPLETED'`.
/// A CANCELLED exchange with that number must be refused with the same error as an unknown number.
#[tokio::test]
async fn non_completed_exchange_is_refused() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    // Drop the check constraint so we can test the SQL WHERE status = 'COMPLETED' condition
    db.exec("ALTER TABLE transactions DROP CONSTRAINT transactions_completed_document_ref_check")
        .await;

    let doc_ref = "XC-20261010-000004";
    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        Some(doc_ref),
        "sub-cancelled-004",
        "CANCELLED",
        d("1000.00"),
        &[d("1000.00")],
    )
    .await;

    let saved = SavedExchange {
        cash: true,
        submission_id: "sub-cancelled-004".to_owned(),
        document_ref: doc_ref.to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("10.00"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    let res = service
        .after_save_in(&mut tx, &branch("MAIN"), &saved, "owner", Utc::now())
        .await;
    tx.rollback().await.unwrap();

    assert!(
        matches!(res, Err(AppError::Internal(_))),
        "non-COMPLETED (CANCELLED) exchange must be refused as not found: {res:?}"
    );

    db.finish().await;
}

/// §9 edge case 1: A completed cash exchange: the same THB as today for the compat-saved exchange.
#[tokio::test]
async fn compat_saved_completed_cash_exchange_records_same_thb() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    let doc_ref = "XC-20261010-000005";
    // Compat-saved exchange: wide principal and line thb_amount are equal (3210.00)
    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        Some(doc_ref),
        "sub-compat-005",
        "COMPLETED",
        d("3210.00"),
        &[d("3210.00")],
    )
    .await;

    let saved = SavedExchange {
        cash: true,
        submission_id: "sub-compat-005".to_owned(),
        document_ref: doc_ref.to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("32.10"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    service
        .after_save_in(&mut tx, &branch("MAIN"), &saved, "owner", Utc::now())
        .await
        .expect("after_save_in compat");
    tx.commit().await.unwrap();

    let thb: String = sqlx::query_scalar(
        "SELECT detail->>'thb' FROM device_events WHERE kind = 'COUNT_CONFIRMED' AND ref = $1",
    )
    .bind("sub-compat-005")
    .fetch_one(&db.pool)
    .await
    .expect("read event");
    assert_eq!(d(&thb), d("3210.00"), "same THB for compat-saved exchange");

    db.finish().await;
}

/// §9 edge case 2: An unknown number: the same error as today.
#[tokio::test]
async fn unknown_document_number_returns_same_error() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    let saved = SavedExchange {
        cash: true,
        submission_id: "sub-unknown-006".to_owned(),
        document_ref: "XC-UNKNOWN-999999".to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("32.00"),
        passport_key: None,
        slip_key: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    let res = service
        .after_save_in(&mut tx, &branch("MAIN"), &saved, "owner", Utc::now())
        .await;
    tx.rollback().await.unwrap();

    assert!(
        matches!(res, Err(AppError::Internal(_))),
        "unknown number must return AppError::Internal: {res:?}"
    );

    db.finish().await;
}

/// §8 & §9: Unfinished exchanges (DRAFT, CONFIRMED) have no number (TX-2's CHECK),
/// so they are never found, as an unknown number is today.
#[tokio::test]
async fn unfinished_exchanges_have_no_number_and_are_never_found() {
    let db = TestDb::start().await;
    let service = build_service(&db);

    // Seed DRAFT and CONFIRMED without document_ref
    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        None,
        "sub-draft-007",
        "DRAFT",
        d("1000.00"),
        &[d("1000.00")],
    )
    .await;

    seed_transaction(
        &db,
        ORG_1,
        "MAIN",
        None,
        "sub-confirmed-008",
        "CONFIRMED",
        d("1000.00"),
        &[d("1000.00")],
    )
    .await;

    // Looking up by submission_id as document_ref
    for sub in ["sub-draft-007", "sub-confirmed-008"] {
        let saved = SavedExchange {
            cash: true,
            submission_id: sub.to_owned(),
            document_ref: sub.to_owned(),
            currency: "USD".to_owned(),
            amount: d("100"),
            rate: d("10.00"),
            passport_key: None,
            slip_key: None,
        };
        let mut tx = db.pool.begin().await.unwrap();
        let res = service
            .after_save_in(&mut tx, &branch("MAIN"), &saved, "owner", Utc::now())
            .await;
        tx.rollback().await.unwrap();

        assert!(
            matches!(res, Err(AppError::Internal(_))),
            "unfinished exchange {sub} has no number and is never found: {res:?}"
        );
    }

    db.finish().await;
}
