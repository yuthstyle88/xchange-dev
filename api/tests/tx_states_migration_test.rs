//! Ticket #670 (TX-2) — migration 2 (`api/migrations/20261014110000_transaction_states.sql`) against
//! a real PostgreSQL: the new `transactions` columns and CHECKs, the three child tables, the backfill
//! of every existing row, the per-organisation number (D-4), the `bookings` foreign key and the
//! `counters` rename.
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §2 D-2/D-3/D-4, §3.2 (the added columns and
//! CHECKs), §3.3 `transaction_lines`, §3.4 `transaction_payments`, §3.5 `receipts`, §8 item 2 (the
//! backfill, verbatim: `settle_thb` = `paid` for BUY and `received` for SELL, `slip_file_key` from
//! `transaction_files kind='SLIP'`, the receipt from `kind='RECEIPT'`, `booking_id` from
//! `bookings.transaction_ref`, `completed_at = recorded_at`; a row without a RECEIPT file makes the
//! migration RAISE). Lane controller's rulings on #670 (2026-10-10, claude/608dff):
//! - D-4: the counter key becomes `document-<organization_id>-<LIVE|DEMO>-<YYYYMMDD>`, and migration
//!   2 renames every existing `document-<mode>-<date>` row of `counters` to its organisation's key,
//!   keeping the value, so each day's sequence carries on;
//! - `bookings.transaction_ref` stops referencing the single-column UNIQUE on `document_ref`; it
//!   becomes `(organization_id, transaction_ref) → transactions (organization_id, document_ref)`.
//!
//! Lane controller's ruling Q3 on #672 (2026-10-10, claude/5edf7bb5, issuecomment-6094343896):
//! migration 2 also adds `kiosk_request_id text NULL`, `staff_draft_id text NULL` and
//! `count_confirmed boolean NOT NULL DEFAULT false` to `transactions` — the save extras are stored
//! at create, and confirm and complete rebuild them from the stored row (D-7). Existing rows get
//! NULL / NULL / false; no backfill.
//!
//! **Migration 2 is found by its file name** (`…_transaction_states.sql`), not its version (lane
//! controller's ruling on #670, 2026-10-10, issuecomment-6094405585): the design's
//! `20261014110000` sorts before CI-4's `20261019100000_customers_org.sql`, which refuses to run over
//! any existing customer (U4), so the file is renamed past main's newest migration when it is pushed
//! (as #675's was). Like `reversals_migration_test.rs`, the backfill tests apply every migration
//! older than migration 2, write rows the way the pre-TX-2 binary writes them, then apply the rest.
//!
//! Customers: CI-4 #651 made a customer belong to the organisation. Every test seeds that shape
//! (`demo_customer`, `org_2`) — once migration 2 sorts after CI-4, it is also the shape that exists
//! just before migration 2.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own (`quote_common::TestDb`).
//!
//! `down` is not run: like every migration in `api/migrations/`, its `down` is a comment in the file
//! (sqlx applies no down section from a plain `.sql` file).
//!
//! Signatures this test assumes ([required]):
//! - exactly one migration is named `…_transaction_states.sql`, and it sorts after CI-4's
//!   `…_customers_org.sql` (§8 item 2: renamed past main's newest at push);
//! - table and column names exactly as §3.2–§3.5; constraint names are not pinned: each rule is
//!   pinned by the row it refuses (SQLSTATE `23514` CHECK, `23503` foreign key, `23505` unique).

mod quote_common;

use std::path::Path;

use quote_common::*;
use sqlx::migrate::Migrator;

const CHECK: &str = "23514";
const FOREIGN_KEY: &str = "23503";
const UNIQUE: &str = "23505";

const ORG_2: &str = "00000000-0000-0000-0000-0000000000b2";

async fn migrator() -> Migrator {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    Migrator::new(dir.as_path())
        .await
        .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
}

/// Design §8 item 2: migration 2's version, found by its file name `…_transaction_states.sql`.
/// It must sort after CI-4's `…_customers_org.sql`, which refuses to run over existing customers.
async fn migration_2() -> i64 {
    let m = migrator().await;
    let named = |suffix: &str| -> Vec<i64> {
        m.migrations
            .iter()
            .filter(|x| x.description.ends_with(suffix))
            .map(|x| x.version)
            .collect()
    };
    let found = named("transaction states");
    assert_eq!(
        found.len(),
        1,
        "exactly one migration named …_transaction_states.sql (design §8 item 2): {found:?}"
    );
    let ci4 = named("customers org");
    assert!(
        ci4.iter().all(|v| *v < found[0]),
        "migration 2 ({}) sorts after CI-4's …_customers_org.sql ({ci4:?}): rename it past main's \
         newest migration (§8 item 2)",
        found[0]
    );
    found[0]
}

/// Apply every migration not yet applied; the error text when one fails.
async fn migrate_rest(db: &TestDb) -> Result<(), String> {
    migration_2().await;
    migrator()
        .await
        .run(&db.pool)
        .await
        .map_err(|e| e.to_string())
}

/// A schema migrated through every migration older than migration 2.
async fn before_migration_2() -> TestDb {
    let version = migration_2().await;
    let newest_before = migrator()
        .await
        .migrations
        .iter()
        .map(|m| m.version)
        .filter(|v| *v < version)
        .max()
        .expect("a migration before migration 2");
    let db = TestDb::empty().await;
    db.migrate_through(Some(newest_before)).await;
    db
}

async fn file(db: &TestDb, key: &str) {
    db.exec(&format!(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('{key}', 'MAIN', 'owner', 'application/pdf', 'sha-{key}', now())"
    ))
    .await;
}

/// A DEMO customer `CU-1` of MAIN's organisation, in CI-4's shape (the full schema).
async fn demo_customer(db: &TestDb) {
    db.exec(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-1', organization_id, 'CU-1', 'Somchai', 'DEMO', now() FROM branches WHERE id = 'MAIN'",
    )
    .await;
}

/// A finished exchange as the pre-TX-2 save writes it (DEMO: no shift needed).
struct Old {
    number: &'static str,
    side: &'static str,
    code: &'static str,
    amount: &'static str,
    unit_size: &'static str,
    rate: &'static str,
    reference_rate: &'static str,
    principal: &'static str,
    fee: &'static str,
    bank_fee: &'static str,
    received: &'static str,
    paid: &'static str,
    method: &'static str,
    payment_ref: &'static str,
    account: &'static str,
    recorded_at: &'static str,
    paid_at: &'static str,
}

/// A BUY by bank transfer with a slip: the shop pays `paid`.
const BUY: Old = Old {
    number: "TEST-XC-20261006-000001",
    side: "BUY",
    code: "USD",
    amount: "100.00",
    unit_size: "1",
    rate: "32.1",
    reference_rate: "32.05",
    principal: "3210.00",
    fee: "10.00",
    bank_fee: "1.50",
    received: "111.11",
    paid: "3200.00",
    method: "Bank Transfer",
    payment_ref: "TRX-9",
    account: "KBank 1",
    recorded_at: "2026-10-06T03:00:00Z",
    paid_at: "2026-10-06T02:59:00Z",
};

/// A SELL in cash without a slip: the shop receives `received`. JPY per 100 (after RE-4's migration 5).
const SELL: Old = Old {
    number: "TEST-XC-20261006-000002",
    side: "SELL",
    code: "JPY",
    amount: "10000.00",
    unit_size: "100",
    rate: "24.0",
    reference_rate: "NULL",
    principal: "2400.00",
    fee: "5.00",
    bank_fee: "0.00",
    received: "2405.00",
    paid: "222.22",
    method: "Cash",
    payment_ref: "",
    account: "Cash drawer 1",
    recorded_at: "2026-10-06T04:00:00Z",
    paid_at: "2026-10-06T04:00:00Z",
};

/// Insert `t` with its PASSPORT and RECEIPT files, and a SLIP when `slip`; answers the row id.
async fn old_row(db: &TestDb, t: &Old, receipt: bool, slip: bool) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let reference_rate = if t.reference_rate == "NULL" {
        "NULL".to_owned()
    } else {
        format!("{}::numeric", t.reference_rate)
    };
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, \
            payload_hash, customer_id, customer_name, nationality, type, code, amount, unit_size, rate, \
            reference_rate, principal, fee, bank_fee, received, paid, method, payment_ref, account, \
            staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by) \
         VALUES ('{id}', '{number}', 'MAIN', 'DEMO', NULL, 'sub-{number}', 'hash', 'CU-1', 'Somchai', \
            'THA', '{side}', '{code}', {amount}, {unit}, {rate}, {reference_rate}, {principal}, {fee}, \
            {bank_fee}, {received}, {paid}, '{method}', '{payment_ref}', '{account}', 'EMP-001', \
            'WALK-IN', '', '2026-10-06', '{paid_at}', '{recorded_at}', 'owner')",
        number = t.number,
        side = t.side,
        code = t.code,
        amount = t.amount,
        unit = t.unit_size,
        rate = t.rate,
        principal = t.principal,
        fee = t.fee,
        bank_fee = t.bank_fee,
        received = t.received,
        paid = t.paid,
        method = t.method,
        payment_ref = t.payment_ref,
        account = t.account,
        paid_at = t.paid_at,
        recorded_at = t.recorded_at,
    ))
    .await;
    let mut kinds = vec!["PASSPORT"];
    if receipt {
        kinds.push("RECEIPT");
    }
    if slip {
        kinds.push("SLIP");
    }
    for kind in kinds {
        let key = format!("{kind}-{}", t.number);
        file(db, &key).await;
        db.exec(&format!(
            "INSERT INTO transaction_files (transaction_id, kind, file_key) \
             VALUES ('{id}', '{kind}', '{key}')"
        ))
        .await;
    }
    id
}

/// A booking USED by the exchange numbered `number`, in `org`'s branch `branch`.
fn booking_sql(id: &str, org: &str, branch: &str, number: &str) -> String {
    format!(
        "INSERT INTO bookings (id, organization_id, branch_id, request_id, request_hash, name, phone, \
            type, code, amount, rate, status, created_at, created_by, submission_id, transaction_ref) \
         VALUES ('{id}', '{org}', '{branch}', 'req-{id}', 'h', 'Somchai', '0800000000', 'BUY', 'USD', \
            100, 32.1, 'USED', now(), 'owner', 'sub-{id}', '{number}')"
    )
}

/// The text of `cols` (comma-separated SQL expressions, each `text`) in the one row `from` (the
/// rest of the query) selects; `NULL` as `"NULL"`.
async fn row(db: &TestDb, cols: &str, from: &str) -> Vec<String> {
    let sql = format!("SELECT ARRAY[{cols}]::text[] {from}");
    let rows: Vec<Vec<Option<String>>> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.clone()))
        .fetch_all(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    assert_eq!(rows.len(), 1, "exactly one row for {sql}");
    rows.into_iter()
        .next()
        .unwrap()
        .into_iter()
        .map(|v| v.unwrap_or_else(|| "NULL".to_owned()))
        .collect()
}

// ── the shape ────────────────────────────────────────────────────────────────────────────────

/// (column, data_type, numeric precision and scale when numeric, nullable)
type Col = (&'static str, &'static str, Option<(i32, i32)>, bool);
/// (column_name, data_type, numeric_precision, numeric_scale, is_nullable)
type InfoRow = (String, String, Option<i32>, Option<i32>, String);

async fn assert_columns(db: &TestDb, table: &str, want: &[Col]) {
    let have: Vec<InfoRow> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, numeric_precision::int, numeric_scale::int, \
                is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1",
    )
    .bind(table)
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("read the columns of {table}: {e}"));
    assert!(!have.is_empty(), "{table} exists after migration 2");
    for (name, ty, ps, nullable) in want {
        let col = have
            .iter()
            .find(|c| c.0 == *name)
            .unwrap_or_else(|| panic!("{table}.{name}: columns are {have:?}"));
        assert_eq!(col.1, *ty, "{table}.{name} type");
        if let Some((p, s)) = ps {
            assert_eq!(
                (col.2, col.3),
                (Some(*p), Some(*s)),
                "{table}.{name} precision"
            );
        }
        assert_eq!(col.4 == "YES", *nullable, "{table}.{name} nullable");
    }
}

#[tokio::test]
async fn migration_2_adds_the_states_and_creates_the_three_child_tables() {
    let db = TestDb::start().await;
    let num = |p, s| Some((p, s));
    let ts = "timestamp with time zone";
    assert_columns(
        &db,
        "transactions",
        &[
            ("status", "text", None, false),
            ("quote_id", "uuid", None, true),
            ("booking_id", "text", None, true),
            ("completed_at", ts, None, true),
            ("cancelled_at", ts, None, true),
            ("cancel_reason", "text", None, true),
            ("approval_request_id", "uuid", None, true),
            // #672 Q3: the save extras, stored at create
            ("kiosk_request_id", "text", None, true),
            ("staff_draft_id", "text", None, true),
            ("count_confirmed", "boolean", None, false),
            // D-3: NULL until COMPLETED
            ("document_ref", "text", None, true),
        ],
    )
    .await;
    assert_columns(
        &db,
        "transaction_lines",
        &[
            ("id", "uuid", None, false),
            ("organization_id", "uuid", None, false),
            ("transaction_id", "uuid", None, false),
            ("quote_line_id", "uuid", None, true),
            ("shop_side", "text", None, false),
            ("currency", "character", None, false),
            ("denomination_id", "uuid", None, true),
            ("quantity", "numeric", num(20, 4), false),
            ("unit_size", "numeric", num(20, 4), false),
            ("rate_snapshot", "numeric", num(20, 8), false),
            ("reference_rate", "numeric", num(20, 8), true),
            ("thb_amount", "numeric", num(20, 2), false),
            ("fee_thb", "numeric", num(20, 2), false),
            ("settle_thb", "numeric", num(20, 2), false),
            ("cost_basis_thb", "numeric", num(20, 2), true),
            ("realized_margin_thb", "numeric", num(20, 2), true),
            ("line_no", "integer", None, false),
            ("created_at", ts, None, false),
            ("updated_at", ts, None, false),
        ],
    )
    .await;
    assert_columns(
        &db,
        "transaction_payments",
        &[
            ("id", "uuid", None, false),
            ("organization_id", "uuid", None, false),
            ("transaction_id", "uuid", None, false),
            ("payment_no", "integer", None, false),
            ("method", "text", None, false),
            ("reference", "text", None, false),
            ("account", "text", None, false),
            ("slip_file_key", "text", None, true),
            ("amount_thb", "numeric", num(20, 2), false),
            ("bank_fee_thb", "numeric", num(20, 2), false),
            ("paid_at", ts, None, false),
            ("created_at", ts, None, false),
            ("updated_at", ts, None, false),
        ],
    )
    .await;
    assert_columns(
        &db,
        "receipts",
        &[
            ("id", "uuid", None, false),
            ("organization_id", "uuid", None, false),
            ("transaction_id", "uuid", None, false),
            ("number", "text", None, false),
            ("revision", "integer", None, false),
            ("issued_at", ts, None, false),
            ("document_ref", "text", None, false),
            ("created_at", ts, None, false),
            ("updated_at", ts, None, false),
        ],
    )
    .await;
    db.finish().await;
}

// ── the backfill ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn every_existing_row_gets_one_line_one_payment_and_one_receipt_with_its_own_figures() {
    let db = before_migration_2().await;
    demo_customer(&db).await;
    let buy = old_row(&db, &BUY, true, true).await;
    let sell = old_row(&db, &SELL, true, false).await;
    db.exec(&booking_sql("BK-1", ORG_1, "MAIN", BUY.number))
        .await;
    migrate_rest(&db).await.expect("migration 2 applies");

    // the four counts of §8 item 2 match
    for table in ["transaction_lines", "transaction_payments", "receipts"] {
        assert_eq!(
            db.count(&format!("SELECT count(*) FROM {table}")).await,
            2,
            "{table}: one row per existing transaction"
        );
    }

    // the parent: COMPLETED, finished when it was recorded, no quote, its booking
    for (id, old, booking) in [(&buy, &BUY, "BK-1"), (&sell, &SELL, "NULL")] {
        assert_eq!(
            row(
                &db,
                "status, (completed_at = recorded_at)::text, quote_id::text, \
                    coalesce(booking_id, 'NULL'), document_ref, cancelled_at::text, \
                    approval_request_id::text, coalesce(kiosk_request_id, 'NULL'), \
                    coalesce(staff_draft_id, 'NULL'), count_confirmed::text",
                &format!("FROM transactions WHERE id = '{id}'")
            )
            .await,
            vec![
                "COMPLETED".to_owned(),
                "true".to_owned(),
                "NULL".to_owned(),
                booking.to_owned(),
                old.number.to_owned(),
                "NULL".to_owned(),
                "NULL".to_owned(),
                // #672 Q3: no backfill of the save extras
                "NULL".to_owned(),
                "NULL".to_owned(),
                "false".to_owned(),
            ],
            "{}: the parent row",
            old.number
        );
    }

    // the line: the row's figures; settle_thb = paid for BUY, received for SELL
    assert_eq!(
        row(
            &db,
            "organization_id::text, shop_side, currency::text, quantity::text, unit_size::text, \
                rate_snapshot::text, reference_rate::text, thb_amount::text, fee_thb::text, \
                settle_thb::text, quote_line_id::text, denomination_id::text, line_no::text",
            &format!("FROM transaction_lines WHERE transaction_id = '{buy}'")
        )
        .await,
        [
            ORG_1,
            "BUY",
            "USD",
            "100.0000",
            "1.0000",
            "32.10000000",
            "32.05000000",
            "3210.00",
            "10.00",
            "3200.00",
            "NULL",
            "NULL",
            "1"
        ],
        "the BUY line"
    );
    assert_eq!(
        row(
            &db,
            "shop_side, currency::text, quantity::text, unit_size::text, rate_snapshot::text, \
                reference_rate::text, thb_amount::text, fee_thb::text, settle_thb::text",
            &format!("FROM transaction_lines WHERE transaction_id = '{sell}'")
        )
        .await,
        [
            "SELL",
            "JPY",
            "10000.0000",
            "100.0000",
            "24.00000000",
            "NULL",
            "2400.00",
            "5.00",
            "2405.00"
        ],
        "the SELL line"
    );

    // the payment: method, reference, account, the SLIP file, amount = the line's settle_thb
    assert_eq!(
        row(
            &db,
            &format!(
                "organization_id::text, payment_no::text, method, reference, account, \
                    slip_file_key, amount_thb::text, bank_fee_thb::text, \
                    (paid_at = '{paid_at}'::timestamptz)::text",
                paid_at = BUY.paid_at
            ),
            &format!("FROM transaction_payments WHERE transaction_id = '{buy}'")
        )
        .await,
        [
            ORG_1,
            "1",
            "Bank Transfer",
            "TRX-9",
            "KBank 1",
            "SLIP-TEST-XC-20261006-000001",
            "3200.00",
            "1.50",
            "true"
        ],
        "the BUY payment"
    );
    assert_eq!(
        row(
            &db,
            "method, reference, account, slip_file_key, amount_thb::text, bank_fee_thb::text",
            &format!("FROM transaction_payments WHERE transaction_id = '{sell}'")
        )
        .await,
        ["Cash", "", "Cash drawer 1", "NULL", "2405.00", "0.00"],
        "the SELL payment: cash, no slip"
    );

    // the receipt: revision 1, numbered as the transaction, the RECEIPT file, issued when recorded
    for (id, old) in [(&buy, &BUY), (&sell, &SELL)] {
        assert_eq!(
            row(
                &db,
                "r.organization_id::text, r.number, r.revision::text, r.document_ref, \
                    (r.issued_at = t.recorded_at)::text",
                &format!(
                    "FROM receipts r JOIN transactions t ON t.id = r.transaction_id \
                     WHERE r.transaction_id = '{id}'"
                )
            )
            .await,
            vec![
                ORG_1.to_owned(),
                old.number.to_owned(),
                "1".to_owned(),
                format!("RECEIPT-{}", old.number),
                "true".to_owned(),
            ],
            "{}: the receipt",
            old.number
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_row_without_a_receipt_file_stops_migration_2_and_nothing_of_it_is_applied() {
    let db = before_migration_2().await;
    demo_customer(&db).await;
    old_row(&db, &BUY, true, true).await;
    old_row(&db, &SELL, false, false).await;
    let version = migration_2().await;
    let err = migrate_rest(&db)
        .await
        .expect_err("migration 2 RAISEs on a transaction without a RECEIPT file (§8 item 2)");
    assert!(
        err.contains(&format!("migration {version}")),
        "the refusal is migration 2's own, not another migration's: {err}"
    );
    // the migration ran in one transaction: nothing of it is left
    assert_eq!(
        db.count(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = current_schema() \
               AND table_name IN ('transaction_lines', 'transaction_payments', 'receipts')"
        )
        .await,
        0,
        "no child table is left behind"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM _sqlx_migrations WHERE version = {version}"
        ))
        .await,
        0,
        "migration 2 is not recorded as applied"
    );
    db.finish().await;
}

#[tokio::test]
async fn existing_document_counters_continue_under_their_organisations_key() {
    let db = before_migration_2().await;
    db.exec(
        "INSERT INTO counters (key, value) VALUES \
            ('document-LIVE-20261006', 5), ('document-DEMO-20261006', 2), \
            ('document-LIVE-20261007', 1), ('customer-LIVE', 9)",
    )
    .await;
    migrate_rest(&db).await.expect("migration 2 applies");
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT key, value FROM counters ORDER BY key")
        .fetch_all(&db.pool)
        .await
        .expect("read counters");
    assert_eq!(
        rows,
        vec![
            ("customer-LIVE".to_owned(), 9),
            (format!("document-{ORG_1}-DEMO-20261006"), 2),
            (format!("document-{ORG_1}-LIVE-20261006"), 5),
            (format!("document-{ORG_1}-LIVE-20261007"), 1),
        ],
        "each document counter keeps its value under the organisation's key; others are untouched"
    );
    db.finish().await;
}

// ── the CHECKs and keys on a migrated schema ─────────────────────────────────────────────────

/// An INSERT of a DEMO transaction at MAIN (org 1) that every CHECK admits as a DRAFT; `over`
/// replaces columns by name.
fn tx_sql(over: &[(&str, &str)]) -> String {
    let mut cols: Vec<(&str, String)> = vec![
        ("id", "gen_random_uuid()".into()),
        ("organization_id", format!("'{ORG_1}'")),
        ("document_ref", "NULL".into()),
        ("branch_id", "'MAIN'".into()),
        ("mode", "'DEMO'".into()),
        ("submission_id", "gen_random_uuid()::text".into()),
        ("payload_hash", "'hash'".into()),
        ("customer_id", "'CU-1'".into()),
        ("customer_name", "'Somchai'".into()),
        ("nationality", "'THA'".into()),
        ("type", "'BUY'".into()),
        ("code", "'USD'".into()),
        ("amount", "100".into()),
        ("rate", "32.1".into()),
        ("principal", "3210".into()),
        ("fee", "0".into()),
        ("bank_fee", "0".into()),
        ("received", "0".into()),
        ("paid", "3210".into()),
        ("method", "'Cash'".into()),
        ("payment_ref", "''".into()),
        ("account", "'Cash drawer 1'".into()),
        ("staff_id", "'EMP-001'".into()),
        ("booking_no", "'WALK-IN'".into()),
        ("notes", "''".into()),
        ("value_date", "'2026-10-06'".into()),
        ("paid_at", "now()".into()),
        ("recorded_at", "now()".into()),
        ("recorded_by", "'owner'".into()),
        ("status", "'DRAFT'".into()),
        ("completed_at", "NULL".into()),
        ("cancelled_at", "NULL".into()),
        ("cancel_reason", "NULL".into()),
    ];
    for (name, value) in over {
        let col = cols
            .iter_mut()
            .find(|c| c.0 == *name)
            .unwrap_or_else(|| panic!("no column {name} in the fixture"));
        col.1 = (*value).to_owned();
    }
    let names: Vec<&str> = cols.iter().map(|c| c.0).collect();
    let values: Vec<&str> = cols.iter().map(|c| c.1.as_str()).collect();
    format!(
        "INSERT INTO transactions ({}) VALUES ({})",
        names.join(", "),
        values.join(", ")
    )
}

const COMPLETED: &[(&str, &str)] = &[
    ("status", "'COMPLETED'"),
    ("document_ref", "'TEST-XC-20261006-000009'"),
    ("completed_at", "now()"),
];

#[tokio::test]
async fn the_status_checks_tie_the_number_and_the_times_to_the_state() {
    let db = TestDb::start().await;
    demo_customer(&db).await;

    // admitted
    for status in ["DRAFT", "PENDING_APPROVAL", "CONFIRMED"] {
        let v = format!("'{status}'");
        assert_eq!(
            db.outcome(&tx_sql(&[("status", &v)])).await,
            "ok",
            "{status} without a number (D-3)"
        );
    }
    assert_eq!(db.outcome(&tx_sql(COMPLETED)).await, "ok", "COMPLETED");
    assert_eq!(
        db.outcome(&tx_sql(&[
            ("status", "'CANCELLED'"),
            ("cancelled_at", "now()"),
            ("cancel_reason", "'customer left'"),
        ]))
        .await,
        "ok",
        "CANCELLED with its time and reason"
    );

    // refused
    let refused: &[(&str, &[(&str, &str)])] = &[
        ("an unknown status", &[("status", "'EXPIRED'")]),
        (
            "COMPLETED without a number",
            &[("status", "'COMPLETED'"), ("completed_at", "now()")],
        ),
        (
            "COMPLETED without completed_at",
            &[
                ("status", "'COMPLETED'"),
                ("document_ref", "'TEST-XC-20261006-000010'"),
            ],
        ),
        (
            "a DRAFT with a number",
            &[("document_ref", "'TEST-XC-20261006-000011'")],
        ),
        ("a DRAFT with completed_at", &[("completed_at", "now()")]),
        (
            "CANCELLED without a reason",
            &[("status", "'CANCELLED'"), ("cancelled_at", "now()")],
        ),
        (
            "CANCELLED without a time",
            &[("status", "'CANCELLED'"), ("cancel_reason", "'x'")],
        ),
        (
            "a CONFIRMED with a cancel time and reason",
            &[
                ("status", "'CONFIRMED'"),
                ("cancelled_at", "now()"),
                ("cancel_reason", "'x'"),
            ],
        ),
    ];
    for (what, over) in refused {
        assert_eq!(db.outcome(&tx_sql(over)).await, CHECK, "{what}");
    }
    db.finish().await;
}

/// A second organisation with branch `B2` and a DEMO customer `CU-2` there.
async fn org_2(db: &TestDb) {
    db.second_organisation(ORG_2, "ORG2", "B2").await;
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('CU-2', '{ORG_2}', 'CU-2', 'Anan', 'DEMO', now())"
    ))
    .await;
}

fn in_org_2(over: &[(&str, &str)]) -> String {
    let org = format!("'{ORG_2}'");
    let mut all: Vec<(&str, &str)> = vec![
        ("organization_id", org.as_str()),
        ("branch_id", "'B2'"),
        ("customer_id", "'CU-2'"),
    ];
    all.extend_from_slice(over);
    tx_sql(&all)
}

#[tokio::test]
async fn a_number_is_unique_within_its_organisation_only() {
    let db = TestDb::start().await;
    demo_customer(&db).await;
    org_2(&db).await;
    assert_eq!(db.outcome(&tx_sql(COMPLETED)).await, "ok", "org 1");
    assert_eq!(
        db.outcome(&tx_sql(COMPLETED)).await,
        UNIQUE,
        "the same number twice in one organisation"
    );
    assert_eq!(
        db.outcome(&in_org_2(COMPLETED)).await,
        "ok",
        "the same number in another organisation (D-4)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_booking_points_at_a_number_of_its_own_organisation() {
    let db = TestDb::start().await;
    demo_customer(&db).await;
    org_2(&db).await;
    db.exec(&tx_sql(COMPLETED)).await;
    let number = "TEST-XC-20261006-000009";
    assert_eq!(
        db.outcome(&booking_sql("BK-1", ORG_1, "MAIN", number))
            .await,
        "ok",
        "a booking of org 1 used by org 1's exchange"
    );
    assert_eq!(
        db.outcome(&booking_sql("BK-2", ORG_2, "B2", number)).await,
        FOREIGN_KEY,
        "a booking of org 2 cannot point at org 1's number"
    );
    assert_eq!(
        db.outcome(&booking_sql(
            "BK-3",
            ORG_1,
            "MAIN",
            "TEST-XC-20261006-999999"
        ))
        .await,
        FOREIGN_KEY,
        "a booking cannot point at a number nobody has"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_child_tables_refuse_rows_the_design_forbids() {
    let db = TestDb::start().await;
    demo_customer(&db).await;
    file(&db, "R-1").await;
    let id = uuid::Uuid::new_v4().to_string();
    let id_v = format!("'{id}'");
    db.exec(&tx_sql(&[
        ("id", &id_v),
        ("status", "'COMPLETED'"),
        ("document_ref", "'TEST-XC-20261006-000009'"),
        ("completed_at", "now()"),
    ]))
    .await;

    let line = |no: &str, qty: &str, side: &str| {
        format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
                created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{id}', '{side}', 'USD', {qty}, 1, 32.1, 3210, 0, \
                3210, {no}, now(), now())"
        )
    };
    assert_eq!(db.outcome(&line("1", "100", "BUY")).await, "ok", "a line");
    assert_eq!(
        db.outcome(&line("1", "100", "BUY")).await,
        UNIQUE,
        "line_no twice"
    );
    assert_eq!(
        db.outcome(&line("2", "0", "BUY")).await,
        CHECK,
        "quantity 0"
    );
    assert_eq!(
        db.outcome(&line("3", "100", "HOLD")).await,
        CHECK,
        "an unknown side"
    );

    let receipt = |revision: &str, number: &str, file: &str| {
        format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{id}', '{number}', {revision}, now(), '{file}', \
                now(), now())"
        )
    };
    assert_eq!(
        db.outcome(&receipt("1", "TEST-XC-20261006-000009", "R-1"))
            .await,
        "ok",
        "revision 1"
    );
    assert_eq!(
        db.outcome(&receipt("1", "TEST-XC-20261006-000009-R1", "R-1"))
            .await,
        UNIQUE,
        "revision 1 twice"
    );
    assert_eq!(
        db.outcome(&receipt("2", "TEST-XC-20261006-000009", "R-1"))
            .await,
        UNIQUE,
        "a number twice in one organisation"
    );
    assert_eq!(
        db.outcome(&receipt("0", "TEST-XC-20261006-000009-R0", "R-1"))
            .await,
        CHECK,
        "revision 0"
    );
    assert_eq!(
        db.outcome(&receipt("2", "TEST-XC-20261006-000009-R2", "NO-SUCH-FILE"))
            .await,
        FOREIGN_KEY,
        "a receipt document that is not a stored file"
    );

    let payment = |no: &str| {
        format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                amount_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{id}', {no}, 'Cash', 3210, now(), now(), now())"
        )
    };
    assert_eq!(db.outcome(&payment("1")).await, "ok", "a payment");
    assert_eq!(db.outcome(&payment("1")).await, UNIQUE, "payment_no twice");
    db.finish().await;
}
