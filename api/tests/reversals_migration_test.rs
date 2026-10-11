//! Ticket #682 (RV-1+RV-2), Part 1 (RV-1) — migration 1 of the reversal design
//! (`api/migrations/<version>_reversals.sql`) against a real PostgreSQL: the `reversals` table, its
//! CHECKs and the one-live-reversal index, and `receipts.kind`.
//! Written before the implementation by the ticket author for the TX/RV chain (claude/f03345fe, on
//! the lane controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/reversal.md` §2 V-4/V-5, §3.1 (`reversals`: the four stored states,
//! `reason` 1–500, `amount_thb > 0`, `business_date`, `approval_request_id NOT NULL`, the POSTED
//! CHECK, `one_live_reversal_per_transaction ON (original_transaction_id) WHERE status <>
//! 'REJECTED'`), §3.2 (`receipts.kind` DEFAULT 'RECEIPT', CHECK `(kind = 'RECEIPT') = (revision =
//! 1)`), §7 (existing receipts become `RECEIPT`; a receipt whose revision is not 1 makes the
//! migration RAISE).
//!
//! **The migration is found by its file name** (`…_reversals.sql`), not its version: the design's
//! `20261016100000` sorts before main's newest migration, and the lane controller renames such a file
//! after main's newest when it is pushed (as #675 was). The test applies every migration older than it,
//! writes rows, then applies the rest.
//!
//! **`approval_request_id` is NULL-able, but only while REQUESTED** (finding RG6 on #682, decided by
//! the ticket author 2026-10-10). AP-1's `ApprovalService::request` reads the subject's
//! `version_hash` from the reversal row (`crates/xc-approval/src/application/approval_service.rs:122`),
//! so the row exists before its request does. The design's `NOT NULL` (§3.1) cannot hold at insert.
//! `CHECK (status = 'REQUESTED' OR approval_request_id IS NOT NULL)` keeps every decided reversal tied
//! to its request, and the request workflow sets it in the same transaction
//! (`reversal_request_test.rs` checks it).
//!
//! **`posting_id`** is pinned only as `uuid`, NULL-able. Its foreign key to `ledger_postings` is
//! #553's table and is not asserted here (ticket body, finding RG1).
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test works in a schema of its own
//! (`quote_common::TestDb`), and rows are seeded in TX-2's schema (#670's migration 2).
//! Constraint names are not pinned; each rule is pinned by the row it refuses (SQLSTATE `23514`
//! CHECK, `23505` unique, `23503` foreign key, `23502` NOT NULL).

mod quote_common;

use std::borrow::Cow;
use std::path::Path;

use quote_common::*;
use sqlx::migrate::Migrator;

/// (column_name, data_type, numeric_precision, numeric_scale, is_nullable)
type InfoRow = (String, String, Option<i32>, Option<i32>, String);
/// (what, columns to replace, the SQLSTATE expected)
type Refused<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a str);

const CHECK: &str = "23514";
const UNIQUE: &str = "23505";
const FOREIGN_KEY: &str = "23503";

async fn migrator() -> (Migrator, i64) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let migrator = Migrator::new(dir.as_path())
        .await
        .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
    let found: Vec<i64> = migrator
        .migrations
        .iter()
        .filter(|m| m.description.ends_with("reversals"))
        .map(|m| m.version)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one migration named …_reversals.sql (design §7): {found:?}"
    );
    (migrator, found[0])
}

/// A schema migrated through every migration older than the reversal one.
async fn before_it() -> TestDb {
    let db = TestDb::empty().await;
    let (mut m, version) = migrator().await;
    let kept: Vec<_> = m
        .migrations
        .iter()
        .filter(|x| x.version < version)
        .cloned()
        .collect();
    m.migrations = Cow::Owned(kept);
    m.run(&db.pool).await.expect("apply the older migrations");
    db
}

async fn the_rest(db: &TestDb) -> Result<(), String> {
    let (m, _) = migrator().await;
    m.run(&db.pool).await.map_err(|e| e.to_string())
}

/// A DEMO customer, a receipt file, and a COMPLETED exchange at MAIN with receipt revision 1;
/// answers the exchange id.
async fn completed(db: &TestDb, n: u32) -> String {
    let id = uuid::Uuid::new_v4().to_string();
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
    let id = uuid::Uuid::new_v4().to_string();
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

/// An INSERT of a reversal of `original` that every CHECK admits as REQUESTED; `over` replaces
/// columns by name.
fn reversal(original: &str, approval: &str, over: &[(&str, &str)]) -> String {
    let mut cols: Vec<(&str, String)> = vec![
        ("id", "gen_random_uuid()".into()),
        ("organization_id", format!("'{ORG_1}'")),
        ("branch_id", "'MAIN'".into()),
        ("original_transaction_id", format!("'{original}'")),
        ("status", "'REQUESTED'".into()),
        ("reason", "'wrong currency'".into()),
        ("amount_thb", "3210.00".into()),
        ("business_date", "'2026-10-06'".into()),
        ("approval_request_id", format!("'{approval}'")),
        ("requested_by", "'EMP-001'".into()),
        ("requested_at", "now()".into()),
        ("created_at", "now()".into()),
        ("updated_at", "now()".into()),
    ];
    for (name, value) in over {
        match cols.iter_mut().find(|c| c.0 == *name) {
            Some(col) => col.1 = (*value).to_owned(),
            None => cols.push((name, (*value).to_owned())),
        }
    }
    let names: Vec<&str> = cols.iter().map(|c| c.0).collect();
    let values: Vec<&str> = cols.iter().map(|c| c.1.as_str()).collect();
    format!(
        "INSERT INTO reversals ({}) VALUES ({})",
        names.join(", "),
        values.join(", ")
    )
}

#[tokio::test]
async fn the_reversals_columns_are_the_designs() {
    let db = TestDb::start().await;
    let have: Vec<InfoRow> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, numeric_precision::int, numeric_scale::int, \
                is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'reversals'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the columns");
    let ts = "timestamp with time zone";
    for (name, ty, scale, nullable) in [
        ("id", "uuid", None, "NO"),
        ("organization_id", "uuid", None, "NO"),
        ("branch_id", "text", None, "NO"),
        ("original_transaction_id", "uuid", None, "NO"),
        ("status", "text", None, "NO"),
        ("reason", "text", None, "NO"),
        ("amount_thb", "numeric", Some((20, 2)), "NO"),
        ("business_date", "date", None, "NO"),
        ("approval_request_id", "uuid", None, "YES"),
        ("posting_id", "uuid", None, "YES"),
        ("receipt_id", "uuid", None, "YES"),
        ("shift_id", "text", None, "YES"),
        ("requested_by", "text", None, "NO"),
        ("requested_at", ts, None, "NO"),
        ("posted_by", "text", None, "YES"),
        ("posted_at", ts, None, "YES"),
        (
            "realized_margin_reversed_thb",
            "numeric",
            Some((20, 2)),
            "YES",
        ),
        ("cost_difference_thb", "numeric", Some((20, 2)), "YES"),
        ("created_at", ts, None, "NO"),
        ("updated_at", ts, None, "NO"),
    ] {
        let col = have
            .iter()
            .find(|c| c.0 == name)
            .unwrap_or_else(|| panic!("reversals.{name}: columns are {have:?}"));
        assert_eq!(col.1, ty, "reversals.{name} type");
        if let Some((p, s)) = scale {
            assert_eq!((col.2, col.3), (Some(p), Some(s)), "reversals.{name} scale");
        }
        assert_eq!(col.4, nullable, "reversals.{name} nullable");
    }
    db.finish().await;
}

#[tokio::test]
async fn the_reversal_checks_admit_the_four_states_and_refuse_the_rest() {
    let db = TestDb::start().await;
    let tx = completed(&db, 1).await;

    let ok = |over: &'static [(&'static str, &'static str)]| over;
    // REQUESTED, then REJECTED of separate originals (the index is tested below)
    for (n, status) in [(2, "REQUESTED"), (3, "APPROVED"), (4, "REJECTED")] {
        let t = completed(&db, n).await;
        let a = request(&db).await;
        let v = format!("'{status}'");
        assert_eq!(
            db.outcome(&reversal(&t, &a, &[("status", &v)])).await,
            "ok",
            "{status}"
        );
    }
    let _ = ok;

    let a = request(&db).await;
    let refused: &[Refused] = &[
        ("an unknown status", &[("status", "'EXPIRED'")], CHECK),
        ("an empty reason", &[("reason", "''")], CHECK),
        (
            "a reason of 501 characters",
            &[("reason", "repeat('x', 501)")],
            CHECK,
        ),
        ("amount 0", &[("amount_thb", "0")], CHECK),
        (
            "an APPROVED reversal without its approval request",
            &[("status", "'APPROVED'"), ("approval_request_id", "NULL")],
            CHECK,
        ),
        (
            "an approval request that does not exist",
            &[("approval_request_id", "gen_random_uuid()")],
            FOREIGN_KEY,
        ),
        (
            "POSTED without its posting, document and time",
            &[("status", "'POSTED'")],
            CHECK,
        ),
        (
            "a REQUESTED one that says it was posted",
            &[
                ("posting_id", "gen_random_uuid()"),
                ("receipt_id", "(SELECT id FROM receipts LIMIT 1)"),
                ("posted_at", "now()"),
            ],
            CHECK,
        ),
    ];
    for (what, over, code) in refused {
        assert_eq!(db.outcome(&reversal(&tx, &a, over)).await, *code, "{what}");
    }
    assert_eq!(
        db.outcome(&reversal(&tx, &a, &[("reason", "repeat('x', 500)")]))
            .await,
        "ok",
        "a reason of 500 characters"
    );
    db.finish().await;
}

#[tokio::test]
async fn one_live_reversal_per_transaction_and_a_rejected_one_allows_another() {
    let db = TestDb::start().await;
    let tx = completed(&db, 1).await;
    let (a1, a2, a3, a4) = (
        request(&db).await,
        request(&db).await,
        request(&db).await,
        request(&db).await,
    );
    assert_eq!(
        db.outcome(&reversal(&tx, &a1, &[])).await,
        "ok",
        "the first"
    );
    assert_eq!(
        db.outcome(&reversal(&tx, &a2, &[])).await,
        UNIQUE,
        "V-5: a second live reversal of one transaction"
    );
    db.exec("UPDATE reversals SET status = 'REJECTED'").await;
    assert_eq!(
        db.outcome(&reversal(&tx, &a3, &[])).await,
        "ok",
        "a REJECTED one is followed by a new request"
    );
    assert_eq!(
        db.outcome(&reversal(&tx, &a4, &[("status", "'APPROVED'")]))
            .await,
        UNIQUE,
        "an APPROVED one is live too"
    );
    db.finish().await;
}

#[tokio::test]
async fn existing_receipts_become_kind_receipt_and_the_kind_follows_the_revision() {
    let db = before_it().await;
    let tx = completed(&db, 1).await;
    the_rest(&db).await.expect("the reversal migration applies");
    assert_eq!(
        db.text(&format!(
            "SELECT kind FROM receipts WHERE transaction_id = '{tx}' AND revision = 1"
        ))
        .await
        .as_deref(),
        Some("RECEIPT"),
        "§7: every existing receipt is a sale's"
    );
    let insert = |revision: u32, kind: &str| {
        format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, kind, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{tx}', 'TEST-XC-20261006-000001-R{revision}', \
                {revision}, now(), 'R-1', '{kind}', now(), now())"
        )
    };
    assert_eq!(
        db.outcome(&insert(2, "REVERSAL")).await,
        "ok",
        "V-4: revision 2, a reversal document"
    );
    assert_eq!(
        db.outcome(&insert(3, "RECEIPT")).await,
        CHECK,
        "a RECEIPT is revision 1 only"
    );
    assert_eq!(
        db.outcome(&insert(3, "REFUND")).await,
        CHECK,
        "an unknown kind"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_receipt_revision_other_than_1_stops_the_migration() {
    let db = before_it().await;
    let tx = completed(&db, 1).await;
    db.exec(&format!(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
            document_ref, created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{tx}', 'TEST-XC-20261006-000001-R2', 2, now(), 'R-1', \
            now(), now())"
    ))
    .await;
    the_rest(&db)
        .await
        .expect_err("§7: a revision other than 1 before this design makes it RAISE");
    assert_eq!(
        db.count(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'reversals'"
        )
        .await,
        0,
        "nothing of it is applied"
    );
    db.finish().await;
}
