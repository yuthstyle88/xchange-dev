//! Row Q1 (`docs/design/xchange-v3.md` §5.5.3): the two time columns of a kiosk request,
//! `kiosk_requests.started_at` and `completed_at`, and who writes them. Written before the
//! implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Part one is the migration's data-migration section, item by item:
//! 1. existing rows keep every column and get no time (both NULL);
//! 2. no backfill: the design's count of rows that will show no time is what it says;
//! 3. deploy order: the older binary's statements work on the new schema;
//! 4. what `down` loses: nothing else depends on the two columns.
//!
//! Part two is the stamping, read straight from the table (the 10-06 bundle's
//! `queue-work.mjs:8`, `:11`, `:30`): a claim stamps its start, the holder's repeat claim
//! does not move it, a refused claim stamps nothing, a completion stamps its time once.
//! These call the service as it is called today and ignore what it answers, so this file
//! compiles on `main` and fails there on results.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod kiosk_service_common;

use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use kiosk_service_common::{
    FakePorts, MAIN, TestDb, at, branch, bring, exact, front1, front2, link, unique,
};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};

/// The migration of row Q1 (§5.5.3, [required] name).
const MIGRATION: &str = "20261008110000_kiosk_work_times.sql";
const MIGRATION_VERSION: i64 = 20261008110000;

/// The older binary's insert, with its column list (`kiosk_repository.rs:211` at `f72b979`).
const OLD_INSERT: &str = "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, \
     side, currency, rate, foreign_amount, thb, input_amount, payment_method, client_ip, \
     submission_id, document_ref) \
     VALUES ($1, $2, $3, $4, $5, 'BUY', 'USD', 32.1, 100, 3210, 100, 'Cash', '10.0.0.1', $6, $7)";
/// The older binary's read and write of a request (`kiosk_repository.rs:22-36`, `:240`).
const OLD_SELECT: &str = "SELECT id, status, created_at, staff, side, currency::text AS currency, \
     rate, foreign_amount, thb, input_amount, payment_method, submission_id, document_ref \
     FROM kiosk_requests WHERE branch_id = $1 AND id = $2";
const OLD_UPDATE: &str = "UPDATE kiosk_requests SET status = $3, staff = $4, submission_id = $5, \
     document_ref = $6 WHERE branch_id = $1 AND id = $2";

fn migrations_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// A schema of its own, migrated up to (not including) the migration of row Q1.
struct Staged {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
}

impl Staged {
    async fn before_q1() -> Staged {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL names a database whose name ends in `_test`");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("q1mig");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        migrator
            .run_to(MIGRATION_VERSION - 1, &pool)
            .await
            .expect("migrate up to the one before Q1");
        Staged {
            pool,
            schema,
            migrator,
        }
    }

    async fn the_rest(&self) {
        self.migrator
            .run(&self.pool)
            .await
            .expect("apply the remaining migrations");
    }

    async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the schema");
        admin.close().await;
    }
}

/// One request of every stored status, written as the older binary writes them.
const OLD_ROWS: [(&str, &str, Option<&str>, Option<&str>); 5] = [
    ("r-quoted", "QUOTED", None, None),
    ("r-waiting", "WAITING", None, None),
    ("r-accepted", "ACCEPTED", Some("emp-1"), None),
    ("r-checked", "ID_CHECKED", Some("emp-1"), None),
    ("r-completed", "COMPLETED", Some("emp-2"), Some("S-1")),
];

async fn insert_old_rows(pool: &PgPool) {
    for (i, (id, status, staff, submission)) in OLD_ROWS.iter().enumerate() {
        sqlx::query(OLD_INSERT)
            .bind(MAIN)
            .bind(id)
            .bind(status)
            .bind(exact(i as i64 * 1000))
            .bind(staff)
            .bind(submission)
            .bind(submission.map(|s| format!("XC-{s}")))
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("insert {id} as the older binary does: {e}"));
    }
}

/// Every column the older binary knows, of every row, as JSON text, in id order.
async fn old_columns(pool: &PgPool) -> Value {
    let rows = sqlx::query(
        // `unit_size` is added later by RU-1's migration (#595, `20261010100000_rate_units.sql`) and is
        // checked by `xc-rate/tests/currency_register_test.rs`; it is not a column the older binary knows.
        // `organization_id` is OR-1's (#615), checked by `xc-rate/tests/organizations_migration_test.rs`.
        // `published_rate_line_id` is RE-4's migration 5 (#601, `20261010125000_published_rate_line_ids.sql`),
        // checked by `api/tests/rates_re4_migration_test.rs`.
        "SELECT (to_jsonb(r) - 'started_at' - 'completed_at' - 'unit_size' - 'organization_id' \
                 - 'published_rate_line_id')::text AS row \
         FROM kiosk_requests r ORDER BY branch_id, id",
    )
    .fetch_all(pool)
    .await
    .expect("read kiosk_requests");
    Value::Array(
        rows.iter()
            .map(|r| serde_json::from_str(&r.get::<String, _>("row")).expect("a row as JSON"))
            .collect(),
    )
}

async fn times(pool: &PgPool, id: &str) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    sqlx::query_as(
        "SELECT started_at, completed_at FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
    )
    .bind(MAIN)
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("read the times of {id}: {e}"))
}

// ── part one: the migration ──────────────────────────────────────────────────

/// The file is where the design says, it sorts after every migration that is on `main`
/// today (it changes a table the kiosk migration made), and it adds exactly two columns:
/// `timestamptz`, nullable, without a default.
#[tokio::test(flavor = "multi_thread")]
async fn the_migration_adds_two_nullable_times_and_nothing_else() {
    let names: Vec<String> = std::fs::read_dir(migrations_dir())
        .expect("api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        names.iter().any(|n| n == MIGRATION),
        "api/migrations/{MIGRATION} exists: {names:?}"
    );
    assert!(
        MIGRATION > "20261007170000_kiosk.sql",
        "{MIGRATION} runs after the kiosk table exists"
    );

    let db = TestDb::start().await;
    let columns: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text, column_default::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'kiosk_requests' \
           AND column_name IN ('started_at', 'completed_at') ORDER BY column_name",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the columns");
    assert_eq!(
        columns,
        vec![
            (
                "completed_at".to_owned(),
                "timestamp with time zone".to_owned(),
                "YES".to_owned(),
                None
            ),
            (
                "started_at".to_owned(),
                "timestamp with time zone".to_owned(),
                "YES".to_owned(),
                None
            ),
        ]
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'kiosk_requests'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count the columns");
    assert_eq!(
        count, 20,
        "the fifteen columns of the kiosk table, the two new ones, RU-1's unit_size (#595), OR-1's organization_id (#615) and RE-4's published_rate_line_id (#601)"
    );
    db.finish().await;
}

/// Data migration item 1: rows written before the migration keep every column as it was,
/// and both times are NULL — unknown times stay unknown (v2: `queue-work.sql`, "do not
/// invent timestamps").
#[tokio::test(flavor = "multi_thread")]
async fn existing_rows_keep_everything_and_get_no_time() {
    let staged = Staged::before_q1().await;
    insert_old_rows(&staged.pool).await;
    let before = old_columns(&staged.pool).await;
    staged.the_rest().await;
    assert_eq!(
        old_columns(&staged.pool).await,
        before,
        "no column the older binary knows changed"
    );
    for (id, ..) in OLD_ROWS {
        assert_eq!(times(&staged.pool, id).await, (None, None), "{id}");
    }
    staged.finish().await;
}

/// Data migration item 2: no backfill. The design's query, run before deploy, counts the
/// rows that will show no time; after the migration exactly those (and the two that never
/// had a holder) are without one.
#[tokio::test(flavor = "multi_thread")]
async fn no_backfill_and_the_designs_count_is_right() {
    let staged = Staged::before_q1().await;
    insert_old_rows(&staged.pool).await;
    let will_show_none: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kiosk_requests WHERE status IN ('ACCEPTED','ID_CHECKED','COMPLETED')",
    )
    .fetch_one(&staged.pool)
    .await
    .expect("the design's count");
    assert_eq!(will_show_none, 3);
    staged.the_rest().await;
    let without: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kiosk_requests WHERE started_at IS NULL AND completed_at IS NULL",
    )
    .fetch_one(&staged.pool)
    .await
    .expect("count rows without times");
    assert_eq!(without, 5, "nothing was filled in");
    staged.finish().await;
}

/// Data migration item 3: the older binary keeps working on the new schema — its insert
/// names its columns, its read names its columns, and its update of a request leaves the
/// times alone.
#[tokio::test(flavor = "multi_thread")]
async fn the_older_binarys_statements_work_on_the_new_schema() {
    let db = TestDb::start().await;
    insert_old_rows(&db.pool).await;
    let row = sqlx::query(OLD_SELECT)
        .bind(MAIN)
        .bind("r-accepted")
        .fetch_one(&db.pool)
        .await
        .expect("the older read");
    assert_eq!(row.get::<String, _>("status"), "ACCEPTED");

    sqlx::query(
        "UPDATE kiosk_requests SET started_at = $2, completed_at = $3 WHERE id = $1 AND branch_id = 'MAIN'",
    )
    .bind("r-checked")
    .bind(exact(5000))
    .bind(exact(9000))
    .execute(&db.pool)
    .await
    .expect("a request with both times");
    sqlx::query(OLD_UPDATE)
        .bind(MAIN)
        .bind("r-checked")
        .bind("COMPLETED")
        .bind("emp-1")
        .bind("S-9")
        .bind("XC-9")
        .execute(&db.pool)
        .await
        .expect("the older update");
    assert_eq!(
        times(&db.pool, "r-checked").await,
        (Some(exact(5000)), Some(exact(9000))),
        "the older update keeps the times"
    );
    db.finish().await;
}

/// Data migration item 4: dropping the two columns by hand needs no `CASCADE` — no index,
/// view or constraint is built on them — and leaves the older binary's statements working.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_two_columns_loses_only_them() {
    let db = TestDb::start().await;
    insert_old_rows(&db.pool).await;
    let before = old_columns(&db.pool).await;
    let mut tx = db.pool.begin().await.expect("begin");
    sqlx::query("ALTER TABLE kiosk_requests DROP COLUMN started_at, DROP COLUMN completed_at")
        .execute(&mut *tx)
        .await
        .expect("drop the two columns without CASCADE");
    sqlx::query(OLD_INSERT)
        .bind(MAIN)
        .bind("r-after")
        .bind("WAITING")
        .bind(exact(10_000))
        .bind(None::<&str>)
        .bind(None::<&str>)
        .bind(None::<String>)
        .execute(&mut *tx)
        .await
        .expect("the older insert after the drop");
    tx.rollback().await.expect("roll back");
    assert_eq!(old_columns(&db.pool).await, before);
    db.finish().await;
}

// ── part two: who writes the times ───────────────────────────────────────────

/// A claim stamps its start, cut to the millisecond (`queue-work.mjs:8`); the holder's
/// repeat claim, before and after the passport check, keeps it and writes no audit line
/// (`queue-work.mjs:11`, §5.5.3 `accept`).
#[tokio::test(flavor = "multi_thread")]
async fn a_claim_stamps_its_start_once() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "waiting").await;
    assert_eq!(
        times(&db.pool, "q1").await,
        (None, None),
        "waiting: no time"
    );

    let answer = service.accept(&main, &front1(), "q1", at(5000)).await;
    assert!(answer.is_ok(), "{}: {answer:?}", "the claim");
    assert_eq!(times(&db.pool, "q1").await, (Some(exact(5000)), None));
    let lines = db.count("test_audit").await;

    let answer = service.accept(&main, &front1(), "q1", at(7000)).await;
    assert!(
        answer.is_ok(),
        "{}: {answer:?}",
        "the holder claims again while ACCEPTED"
    );
    assert_eq!(
        times(&db.pool, "q1").await,
        (Some(exact(5000)), None),
        "the start does not move"
    );
    service
        .identity_checked(&main, &front1(), "q1", true, at(8000))
        .await
        .expect("the passport check");
    let lines = lines + 1;
    let answer = service.accept(&main, &front1(), "q1", at(9000)).await;
    assert!(
        answer.is_ok(),
        "{}: {answer:?}",
        "the holder claims again while ID_CHECKED"
    );
    assert_eq!(times(&db.pool, "q1").await, (Some(exact(5000)), None));
    assert_eq!(
        db.count("test_audit").await,
        lines,
        "a repeat claim writes no audit line"
    );
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("ID_CHECKED"));
    db.finish().await;
}

/// A claim that is not taken — another holder, or nothing to claim — stamps nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_claim_not_taken_stamps_nothing() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "accepted").await;
    bring(&service, MAIN, "q2", "quoted").await;
    let _ = service.accept(&main, &front2(), "q1", at(9000)).await;
    let _ = service.accept(&main, &front2(), "q2", at(9000)).await;
    let _ = service.accept(&main, &front2(), "nope", at(9000)).await;
    assert_eq!(times(&db.pool, "q1").await, (Some(exact(5000)), None));
    assert_eq!(times(&db.pool, "q2").await, (None, None));
    assert_eq!(db.rows().await[0]["staff"], json!("front1"));
    db.finish().await;
}

/// The save of the exchange completes the request and stamps the time once, on the save's
/// own transaction; a repeat of the same save never rewrites it (`queue-work.mjs:30`,
/// `COALESCE`). A save that is rolled back leaves no time.
#[tokio::test(flavor = "multi_thread")]
async fn a_completion_stamps_its_time_once() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "checked").await;

    let mut tx = db.begin().await;
    service
        .complete_in(
            &mut tx,
            &main,
            "q1",
            &link("S-1", "XC-1"),
            &front1(),
            at(20_000),
        )
        .await
        .expect("complete, then roll back");
    tx.rollback().await.expect("roll back");
    assert_eq!(times(&db.pool, "q1").await, (Some(exact(5000)), None));

    let mut tx = db.begin().await;
    service
        .complete_in(
            &mut tx,
            &main,
            "q1",
            &link("S-1", "XC-1"),
            &front1(),
            at(30_000),
        )
        .await
        .expect("complete");
    tx.commit().await.expect("commit");
    assert_eq!(
        times(&db.pool, "q1").await,
        (Some(exact(5000)), Some(exact(30_000)))
    );

    let mut tx = db.begin().await;
    service
        .complete_in(
            &mut tx,
            &main,
            "q1",
            &link("S-1", "XC-1"),
            &front1(),
            at(40_000),
        )
        .await
        .expect("the same save again");
    tx.commit().await.expect("commit");
    assert_eq!(
        times(&db.pool, "q1").await,
        (Some(exact(5000)), Some(exact(30_000))),
        "a repeat keeps the first completion"
    );
    db.finish().await;
}
