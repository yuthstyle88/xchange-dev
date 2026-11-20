//! The shared admin transaction row (`dto/v3/transaction_row.rs`) names the staff member and says when a
//! transaction was reversed.
//!
//! Symptom (seen 2026-11-20 on a local run while writing `docs/USER_MANUAL.md`): the admin's transaction
//! dialog showed a reversed SELL as `สำเร็จ`, and its `พนักงาน` row showed a user UUID.
//! Root cause: `ROW_COLUMNS` / `ROW_FROM`, the one select every admin list view uses (`admin_transactions`,
//! `admin_slips`, `admin_overview`, `admin_customer_history`), read `t.status` and `t.staff_id` only. By the
//! reversal design (`docs/design/blueprint/reversal.md` V-2) the original row stays `COMPLETED` and
//! "reversed" is read from its POSTED reversal; and `staff_id` is a `users.id`, whose name lives in `users`.
//!
//! | Case | `staffName` | `reversedAt` |
//! |---|---|---|
//! | served by a user with a name, reversal POSTED | that name | the reversal's `posted_at` |
//! | `staff_id` that is no user (legacy employee id) | `None` (screens fall back to `staff`) | `None` |
//! | reversal only REQUESTED | – | `None` |
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each run works in a schema of its own.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xchange::presentation::dto::v3::transaction_row::{
    ROW_COLUMNS, ROW_FROM, TransactionRow, TxSqlRow,
};

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG: &str = "00000000-0000-0000-0000-0000000000a1";
const BRANCH: &str = "RW-1";

async fn schema_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip")
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    let database = options.get_database().unwrap_or("").to_owned();
    assert!(
        database.ends_with("_test"),
        "'{database}' does not end in `_test`"
    );
    let schema = xc_testdb::unique_schema_name("tx_row_staff_rev");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(MUST_FINISH)
        .connect_with(options.clone())
        .await
        .expect("connect to the test database");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("create the test schema");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(MUST_FINISH)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    Migrator::new(migrations.as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");
    pool
}

async fn exec(pool: &PgPool, sql: String) {
    sqlx::query(AssertSqlSafe(sql.clone()))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn seed_org(pool: &PgPool) {
    exec(
        pool,
        format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{ORG}', 'RWORG', 'Row Test Org', 'Asia/Bangkok', now()) ON CONFLICT (id) DO NOTHING"
        ),
    )
    .await;
    exec(
        pool,
        format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{BRANCH}', 'Row Branch', '{ORG}') \
             ON CONFLICT DO NOTHING"
        ),
    )
    .await;
}

async fn seed_user(pool: &PgPool, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    exec(
        pool,
        format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{id}', '{ORG}', '{name}', 'ACTIVE', 'test')"
        ),
    )
    .await;
    id
}

/// Runs `sqls` in one transaction with foreign-key triggers off (as `ledger_drop_old_tables_test.rs` does):
/// the shift, customer, approval, posting and receipt rows they point at are not what the row reads. The
/// tables' CHECKs still hold.
async fn exec_unlinked(pool: &PgPool, sqls: Vec<String>) {
    let mut conn = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("foreign-key triggers off");
    for sql in sqls {
        sqlx::query(AssertSqlSafe(sql.clone()))
            .execute(&mut *conn)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    conn.commit().await.expect("commit");
}

/// One COMPLETED cash SELL of 50 USD served by `staff_id`.
async fn seed_tx(pool: &PgPool, staff_id: &str, doc: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = "2026-11-20T02:59:00Z";
    exec_unlinked(
        pool,
        vec![
        format!(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
                recorded_at, recorded_by, status, completed_at, organization_id) \
             VALUES ('{id}', '{doc}', '{BRANCH}', 'LIVE', 'SH-RW', 'sub-{id}', 'hash-{id}', 'C-1', 'Pat Sample', \
                'THA', '{staff_id}', 'WALK-IN', '', '2026-11-20', '{now}', '{now}', '{staff_id}', 'COMPLETED', \
                '{now}', '{ORG}')"
        ),
        format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
                unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{id}', 'SELL', 'USD', 50, 1, 34.20, 1710, 0, 1710, 1, \
                '{now}', '{now}')"
        ),
        format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, reference, \
                account, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), '{ORG}', '{id}', 1, 'Cash', '', 'drawer', 1710, 0, '{now}', '{now}', '{now}')"
        ),
        ],
    )
    .await;
    id
}

/// A reversal of `tx` in `status`. Its approval request, ledger posting and receipt are not part of what
/// the row reads, so they are written with foreign-key triggers off (as `ledger_drop_old_tables_test.rs`
/// does); the table's CHECKs still hold.
async fn seed_reversal(pool: &PgPool, tx: Uuid, status: &str, posted_at: Option<&str>) {
    let (posting, receipt, at) = match posted_at {
        Some(at) => (
            format!("'{}'", Uuid::new_v4()),
            format!("'{}'", Uuid::new_v4()),
            format!("'{at}'"),
        ),
        None => ("NULL".into(), "NULL".into(), "NULL".into()),
    };
    let approval = if status == "REQUESTED" {
        "NULL".to_string()
    } else {
        format!("'{}'", Uuid::new_v4())
    };
    let mut conn = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *conn)
        .await
        .expect("foreign-key triggers off");
    let sql = format!(
        "INSERT INTO reversals (organization_id, branch_id, original_transaction_id, status, reason, amount_thb, \
            business_date, approval_request_id, posting_id, receipt_id, requested_by, requested_at, posted_by, \
            posted_at) \
         VALUES ('{ORG}', '{BRANCH}', '{tx}', '{status}', 'wrong currency', 1710, '2026-11-20', {approval}, \
            {posting}, {receipt}, 'manager', now(), NULL, {at})"
    );
    sqlx::query(AssertSqlSafe(sql.clone()))
        .execute(&mut *conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    conn.commit().await.expect("commit");
}

async fn row(pool: &PgPool, tx: Uuid) -> TransactionRow {
    let sql = format!("SELECT {ROW_COLUMNS} FROM {ROW_FROM} WHERE t.id = $1");
    let r: TxSqlRow = sqlx::query_as(AssertSqlSafe(sql))
        .bind(tx)
        .fetch_one(pool)
        .await
        .expect("the row reads");
    TransactionRow::from_sql_row(r).expect("the row maps")
}

#[tokio::test]
async fn a_reversed_sale_says_when_and_names_who_served() {
    let pool = schema_pool().await;
    seed_org(&pool).await;
    let teller = seed_user(&pool, "Teller Demo").await;
    let tx = seed_tx(&pool, &teller.to_string(), "XC-RW-000001").await;
    seed_reversal(&pool, tx, "POSTED", Some("2026-11-20T03:10:00Z")).await;

    let r = row(&pool, tx).await;
    assert_eq!(
        r.status, "COMPLETED",
        "the original row is never changed (reversal V-2)"
    );
    assert_eq!(r.staff, teller.to_string(), "staff stays the user id");
    assert_eq!(r.staff_name.as_deref(), Some("Teller Demo"));
    let want = Utc.with_ymd_and_hms(2026, 11, 20, 3, 10, 0).unwrap();
    assert_eq!(
        r.reversed_at.as_deref(),
        Some(
            want.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                .as_str()
        )
    );
}

#[tokio::test]
async fn a_requested_reversal_is_not_a_reversal_and_an_unknown_staff_id_has_no_name() {
    let pool = schema_pool().await;
    seed_org(&pool).await;
    let tx = seed_tx(&pool, "EMP-464", "XC-RW-000002").await;
    seed_reversal(&pool, tx, "REQUESTED", None).await;

    let r = row(&pool, tx).await;
    assert_eq!(r.staff, "EMP-464");
    assert_eq!(r.staff_name, None, "no user has that id");
    assert_eq!(r.reversed_at, None, "only a POSTED reversal reverses");
}
