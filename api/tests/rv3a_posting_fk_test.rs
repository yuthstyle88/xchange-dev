//! Ticket #1495 (RV-3a) — `reversals.posting_id` foreign key to `ledger_postings(id)`.
//!
//! Written before the implementation by gemini/pool-4 (test-writer, ordered by claude/ad05bf8d).
//!
//! Design:
//! - Ticket #1495: RV-3a: reversals.posting_id foreign key constraint `reversals_posting_id_fkey`.
//! - Section 7: `[required] constraint name reversals_posting_id_fkey`.
//! - Section 8 & 9 (Behaviour / Edge Cases):
//!   1. A POSTED reversal whose `posting_id` names no `ledger_postings` row must be refused with
//!      SQLSTATE 23503 (`reversals_posting_id_fkey`).
//!   2. A PENDING reversal with NULL `posting_id` is accepted (non-POSTED reversals).
//!   3. A POSTED reversal naming a real `ledger_postings` row is accepted.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test works in a schema of its own
//! (`quote_common::TestDb`), applying all migrations.

mod quote_common;

use quote_common::*;
use uuid::Uuid;

const FOREIGN_KEY: &str = "23503";

/// A DEMO customer, a receipt file, and a COMPLETED exchange at MAIN with receipt revision 1;
/// answers the exchange id.
async fn completed(db: &TestDb, n: u32) -> String {
    let id = Uuid::new_v4().to_string();
    let number = format!("TEST-XC-20261006-{n:06}");
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-{n}', organization_id, 'CU-{n}', 'Somchai', 'DEMO', now() FROM branches \
         WHERE id = 'MAIN' ON CONFLICT DO NOTHING"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('R-{n}', 'MAIN', 'owner', 'text/html', 'sha-{n}', now())"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, submission_id, payload_hash, \
            customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
            bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
            value_date, paid_at, recorded_at, recorded_by, status, completed_at) \
         VALUES ('{id}', '{number}', 'MAIN', 'DEMO', 'sub-{n}', 'h', 'CU-{n}', 'Somchai', 'THA', \
            'BUY', 'USD', 100, 32.1, 3210, 0, 0, 0, 3210, 'Cash', '', 'Cash drawer 1', 'EMP-001', \
            'WALK-IN', '', '2026-10-06', now(), now(), 'owner', 'COMPLETED', now())"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
            document_ref, created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{id}', '{number}', 1, now(), 'R-{n}', now(), now())"
    ))
    .await;
    id
}

/// A PENDING approval request at MAIN; answers its id.
async fn request(db: &TestDb) -> String {
    let id = Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
            object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
            status, idempotency_key) \
         VALUES ('{id}', '{ORG_1}', 'TRANSACTION_REVERSAL', 'MAIN', '{id}', 'h', '1', '{{}}', \
            'EMP-001', now(), 'PENDING', 'k-{id}')"
    ))
    .await;
    id
}

/// A revision 2 receipt with kind 'REVERSAL' for transaction `tx_id`.
async fn reversal_receipt(db: &TestDb, tx_id: &str, n: u32) -> String {
    let id = Uuid::new_v4().to_string();
    let number = format!("TEST-XC-20261006-{n:06}-R2");
    db.exec(&format!(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
            document_ref, kind, created_at, updated_at) \
         VALUES ('{id}', '{ORG_1}', '{tx_id}', '{number}', 2, now(), 'R-{n}', 'REVERSAL', now(), now())"
    ))
    .await;
    id
}

/// A seeded `ledger_postings` row matching the schema of main's ledger migration.
async fn seed_ledger_posting(db: &TestDb, posting_id: &str, source_id: &str) {
    db.exec(&format!(
        "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
         VALUES ('{posting_id}', '{ORG_1}', 'MAIN', 'REVERSAL', '{source_id}', 'reversal', 'POSTED', now(), 'EMP-001')"
    ))
    .await;
}

/// Returns (SQLSTATE, constraint_name) on error, or ("ok", None) on success.
async fn outcome_and_constraint(pool: &sqlx::PgPool, sql: &str) -> (String, Option<String>) {
    match sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
    {
        Ok(_) => ("ok".to_owned(), None),
        Err(e) => match e.as_database_error() {
            Some(d) => (
                d.code().map(|c| c.into_owned()).unwrap_or_default(),
                d.constraint().map(|c| c.to_owned()),
            ),
            None => panic!("{sql}: not a database error: {e}"),
        },
    }
}

/// §9 Row 1 / §4: A POSTED reversal whose `posting_id` names no `ledger_postings` row must be refused
/// with SQLSTATE 23503 (Foreign Key Violation) and constraint name `reversals_posting_id_fkey`.
#[tokio::test]
async fn posted_reversal_naming_nonexistent_posting_fails_with_foreign_key_violation() {
    let db = TestDb::start().await;
    let tx = completed(&db, 1).await;
    let req = request(&db).await;
    let receipt = reversal_receipt(&db, &tx, 1).await;
    let nonexistent_posting = Uuid::new_v4().to_string();

    let sql = format!(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
            reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
            requested_by, requested_at, posted_by, posted_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{tx}', 'POSTED', \
            'wrong currency', 3210.00, '2026-10-06', '{req}', '{nonexistent_posting}', '{receipt}', \
            'EMP-001', now(), 'EMP-001', now())"
    );

    let (code, constraint) = outcome_and_constraint(&db.pool, &sql).await;
    assert_eq!(
        code, FOREIGN_KEY,
        "a POSTED reversal whose posting_id names no ledger_postings row must be refused with SQLSTATE 23503"
    );
    assert_eq!(
        constraint.as_deref(),
        Some("reversals_posting_id_fkey"),
        "foreign key constraint name must be reversals_posting_id_fkey"
    );

    db.finish().await;
}

/// §9 Row 2: NULL `posting_id` is allowed for non-POSTED (PENDING) reversals.
#[tokio::test]
async fn pending_reversal_with_null_posting_id_is_accepted() {
    let db = TestDb::start().await;
    let tx = completed(&db, 2).await;
    let req = request(&db).await;

    let sql = format!(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
            reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
            requested_by, requested_at, posted_by, posted_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{tx}', 'REQUESTED', \
            'wrong currency', 3210.00, '2026-10-06', '{req}', NULL, NULL, \
            'EMP-001', now(), NULL, NULL)"
    );

    assert_eq!(
        db.outcome(&sql).await,
        "ok",
        "a PENDING reversal with NULL posting_id must be accepted"
    );

    db.finish().await;
}

/// §9 Row 3: A POSTED reversal naming a real `ledger_postings` row is accepted.
#[tokio::test]
async fn posted_reversal_naming_real_ledger_posting_is_accepted() {
    let db = TestDb::start().await;
    let tx = completed(&db, 3).await;
    let req = request(&db).await;
    let receipt = reversal_receipt(&db, &tx, 3).await;
    let posting_id = Uuid::new_v4().to_string();

    seed_ledger_posting(&db, &posting_id, &tx).await;

    let sql = format!(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
            reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
            requested_by, requested_at, posted_by, posted_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', '{tx}', 'POSTED', \
            'wrong currency', 3210.00, '2026-10-06', '{req}', '{posting_id}', '{receipt}', \
            'EMP-001', now(), 'EMP-001', now())"
    );

    assert_eq!(
        db.outcome(&sql).await,
        "ok",
        "a POSTED reversal naming a real ledger_postings row must be accepted"
    );

    db.finish().await;
}
