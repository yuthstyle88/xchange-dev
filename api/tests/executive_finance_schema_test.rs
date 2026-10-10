//! Ticket #884 (EX-2): the tables the executive overview (A01) and the finance reconciliation (F01) write, as
//! `docs/design/blueprint/executive-finance.md` §6 defines them. Written before the implementation by the ticket
//! author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `branch_alert_settings`: the per-branch stale-data minutes (owner E-3, E-6); NULL means no alert.
//! - `branch_stock_floors`: the per-branch minimum quantity per currency (E-3, E-6).
//! - `reconciliation_certifications`: F01's certification, append-only (E-8; design §5.2). It has no idempotency
//!   column: the command's idempotency is the shared `idempotency_records` (design §5.2, PR 882 review).
//! - `branch_alert_settings.version`: the expected version a save is checked against (design §6.1).
//!
//! Signatures these tests assume ([required], #884 §7): the three tables with the columns, keys, CHECKs and
//! composite foreign keys of design §6, and a trigger that refuses UPDATE and DELETE on
//! `reconciliation_certifications`.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`); none skips.
//! Each test has a schema of its own with `api/migrations/` applied.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

const MUST_FINISH: Duration = Duration::from_secs(20);

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. The EX-2 schema tests need a real PostgreSQL and do not skip.")
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("ex2s");
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
            .max_connections(4)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    async fn exec(&self, sql: &str) -> Result<u64, sqlx::Error> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected())
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect to drop the test schema");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}

/// The organisation and branch the migrations seed (`MAIN` of organisation `XC`).
const MAIN_ORG: &str = "(SELECT organization_id FROM branches WHERE id = 'MAIN')";

fn alert_row(minutes: &str) -> String {
    format!(
        "INSERT INTO branch_alert_settings (organization_id, branch_id, stale_data_minutes, updated_by, updated_at) \
         VALUES ({MAIN_ORG}, 'MAIN', {minutes}, gen_random_uuid(), now())"
    )
}

fn floor_row(currency: &str, min: &str) -> String {
    format!(
        "INSERT INTO branch_stock_floors (organization_id, branch_id, currency, min_quantity, updated_by, updated_at) \
         VALUES ({MAIN_ORG}, 'MAIN', '{currency}', {min}, gen_random_uuid(), now())"
    )
}

fn certification_row(hash: &str) -> String {
    format!(
        "INSERT INTO reconciliation_certifications \
           (id, organization_id, branch_id, business_date, figures_hash, figures, certified_by, certified_at, note) \
         VALUES (gen_random_uuid(), {MAIN_ORG}, 'MAIN', DATE '2026-10-10', '{hash}', '{{}}'::jsonb, \
                 gen_random_uuid(), now(), NULL)"
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_alert_settings_take_positive_minutes_or_none_and_refuse_zero() {
    let db = TestDb::start().await;
    db.exec(&alert_row("15"))
        .await
        .expect("15 minutes is a valid threshold");
    assert_eq!(db.count("branch_alert_settings").await, 1);
    db.exec("DELETE FROM branch_alert_settings")
        .await
        .expect("clear");
    db.exec(&alert_row("NULL"))
        .await
        .expect("NULL means no stale-data alert (E-3)");
    db.exec("DELETE FROM branch_alert_settings")
        .await
        .expect("clear");
    assert!(
        db.exec(&alert_row("0")).await.is_err(),
        "0 minutes is refused by the CHECK"
    );
    assert!(
        db.exec(&alert_row("-5")).await.is_err(),
        "a negative value is refused"
    );
    assert_eq!(
        db.count("branch_alert_settings").await,
        0,
        "a refused row writes nothing"
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_alert_settings_are_one_row_per_branch_of_its_own_organisation() {
    let db = TestDb::start().await;
    db.exec(&alert_row("15")).await.expect("the first row");
    assert!(
        db.exec(&alert_row("30")).await.is_err(),
        "a second row for the same branch is refused (PK)"
    );
    let unknown = "INSERT INTO branch_alert_settings (organization_id, branch_id, stale_data_minutes, updated_by, \
                   updated_at) VALUES ((SELECT organization_id FROM branches WHERE id = 'MAIN'), 'NO_SUCH', 15, \
                   gen_random_uuid(), now())";
    assert!(
        db.exec(unknown).await.is_err(),
        "a branch that does not exist is refused (composite FK)"
    );
    let other_org = "INSERT INTO branch_alert_settings (organization_id, branch_id, stale_data_minutes, updated_by, \
                     updated_at) VALUES (gen_random_uuid(), 'MAIN', 15, gen_random_uuid(), now())";
    assert!(
        db.exec(other_org).await.is_err(),
        "MAIN under another organisation is refused (composite FK on (organization_id, branch_id))"
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_stock_floors_take_a_registered_currency_and_a_non_negative_quantity() {
    let db = TestDb::start().await;
    db.exec(&floor_row("USD", "1000.0000"))
        .await
        .expect("a USD floor");
    db.exec(&floor_row("JPY", "0"))
        .await
        .expect("a zero floor is allowed");
    assert_eq!(db.count("branch_stock_floors").await, 2);
    assert!(
        db.exec(&floor_row("USD", "5")).await.is_err(),
        "one floor per (branch, currency) (PK)"
    );
    assert!(
        db.exec(&floor_row("EUR", "-1")).await.is_err(),
        "a negative floor is refused"
    );
    assert!(
        db.exec(&floor_row("ZZZ", "1")).await.is_err(),
        "a currency outside the register is refused (FK)"
    );
    assert_eq!(
        db.count("branch_stock_floors").await,
        2,
        "refused rows write nothing"
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certification_can_be_repeated_for_a_day_but_never_changed_or_removed() {
    let db = TestDb::start().await;
    db.exec(&certification_row("hash-1"))
        .await
        .expect("a certification");
    db.exec(&certification_row("hash-1"))
        .await
        .expect("re-certifying the same day appends a second row");
    assert_eq!(db.count("reconciliation_certifications").await, 2);
    let key_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema() \
         AND table_name = 'reconciliation_certifications' AND column_name = 'idempotency_key'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read the table's columns");
    assert_eq!(
        key_columns, 0,
        "idempotency is the shared idempotency_records, not a column here"
    );
    assert!(
        db.exec("UPDATE reconciliation_certifications SET note = 'edited'")
            .await
            .is_err(),
        "UPDATE is refused: certifications are append-only"
    );
    assert!(
        db.exec("DELETE FROM reconciliation_certifications")
            .await
            .is_err(),
        "DELETE is refused: certifications are append-only"
    );
    assert_eq!(
        db.count("reconciliation_certifications").await,
        2,
        "nothing changed"
    );
    let notes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reconciliation_certifications WHERE note IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read notes");
    assert_eq!(notes, 0, "the refused UPDATE left every row as written");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certification_names_a_branch_of_its_organisation() {
    let db = TestDb::start().await;
    let unknown = "INSERT INTO reconciliation_certifications (id, organization_id, branch_id, business_date, \
                   figures_hash, figures, certified_by, certified_at) VALUES (gen_random_uuid(), \
                   (SELECT organization_id FROM branches WHERE id = 'MAIN'), 'NO_SUCH', DATE '2026-10-10', 'h', \
                   '{}'::jsonb, gen_random_uuid(), now())";
    assert!(
        db.exec(unknown).await.is_err(),
        "an unknown branch is refused (composite FK)"
    );
    assert_eq!(db.count("reconciliation_certifications").await, 0);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_alert_settings_start_at_version_one_and_refuse_a_version_below_one() {
    let db = TestDb::start().await;
    db.exec(&alert_row("15"))
        .await
        .expect("a row without a version takes the default");
    let version: i32 =
        sqlx::query_scalar("SELECT version FROM branch_alert_settings WHERE branch_id = 'MAIN'")
            .fetch_one(&db.pool)
            .await
            .expect("branch_alert_settings.version exists (design §6.1)");
    assert_eq!(version, 1);
    assert!(
        db.exec("UPDATE branch_alert_settings SET version = 0")
            .await
            .is_err(),
        "a version below one is refused by the CHECK"
    );
    db.exec("UPDATE branch_alert_settings SET version = version + 1")
        .await
        .expect("a save moves the version");
    let version: i32 =
        sqlx::query_scalar("SELECT version FROM branch_alert_settings WHERE branch_id = 'MAIN'")
            .fetch_one(&db.pool)
            .await
            .expect("read the version");
    assert_eq!(version, 2);
    db.finish().await;
}
