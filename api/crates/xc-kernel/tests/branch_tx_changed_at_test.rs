//! Ticket #884 (EX-2): every version bump also records *when* the branch changed, so A01 can show `as_of` per
//! branch (bp:§11 "แสดง as_of แต่ละสาขา"). Written before the implementation by the ticket author (claude/e5ab6c).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/executive-finance.md` §2 D-4, §6, §10. `branch_versions.changed_at` is set by
//! the same statement that bumps `version` (`branch_tx.rs` `run` and `run_pair`), never by a second clock.
//!
//! Signatures these tests assume ([required], #884 §7):
//! - `branch_versions.changed_at timestamptz NOT NULL DEFAULT now()`;
//! - `BranchTx::{run, run_pair, version}` unchanged.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`); none skips.
//! Each test has a schema of its own with `api/migrations/` applied.

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pos_error::AppError;
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

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

fn branch(label: &str) -> BranchId {
    BranchId::new(unique(label))
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. The changed_at tests need a real PostgreSQL and do not skip.")
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("ex2");
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
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// The database's own clock, outside any transaction, in microseconds since the epoch.
    async fn clock(&self) -> i64 {
        sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000000)::bigint")
            .fetch_one(&self.pool)
            .await
            .expect("read the database clock")
    }

    /// `(version, changed_at)` with the time in microseconds since the epoch.
    async fn row(&self, branch: &BranchId) -> (i64, i64) {
        sqlx::query_as(
            "SELECT version, (extract(epoch FROM changed_at) * 1000000)::bigint \
             FROM branch_versions WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_one(&self.pool)
        .await
        .expect("read the branch_versions row")
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

async fn noop_run(bt: &BranchTx, b: &BranchId) {
    bt.run(b, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("the run commits");
}

/// The column is `NOT NULL` with a default, so the old binary's bump-only statement keeps working.
#[tokio::test(flavor = "multi_thread")]
async fn changed_at_is_a_not_null_timestamptz_with_a_default() {
    let db = TestDb::start().await;
    let (data_type, nullable, default): (String, String, Option<String>) = sqlx::query_as(
        "SELECT data_type, is_nullable, column_default FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'branch_versions' AND column_name = 'changed_at'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("branch_versions.changed_at exists (#884 §7)");
    assert_eq!(data_type, "timestamp with time zone");
    assert_eq!(nullable, "NO");
    assert!(
        default.as_deref().is_some_and(|d| d.contains("now()")),
        "the default is now(): {default:?}"
    );
    db.finish().await;
}

/// A committed `run` raises the version by one and stamps `changed_at` with a time inside the call.
#[tokio::test(flavor = "multi_thread")]
async fn a_committed_run_stamps_changed_at_inside_the_call() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let a = branch("a");

    let before = db.clock().await;
    noop_run(&bt, &a).await;
    let after = db.clock().await;

    let (version, changed_at) = db.row(&a).await;
    assert_eq!(
        version, 1,
        "a branch's first run inserts it at 0 and bumps it once"
    );
    assert!(
        before <= changed_at && changed_at <= after,
        "changed_at {changed_at} lies within the run's call [{before}, {after}]"
    );
    db.finish().await;
}

/// The stamp moves with every bump: a later run gives a later time, not the first one kept.
#[tokio::test(flavor = "multi_thread")]
async fn every_bump_moves_changed_at_forward() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let a = branch("a");

    noop_run(&bt, &a).await;
    let (_, first) = db.row(&a).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let before = db.clock().await;
    noop_run(&bt, &a).await;
    let (version, second) = db.row(&a).await;

    assert_eq!(version, 2);
    assert!(
        second > first,
        "the second bump's time {second} is after the first's {first}"
    );
    assert!(
        second >= before,
        "the second stamp {second} is at or after its call began {before}"
    );
    db.finish().await;
}

/// A committed `run_pair` stamps both rows, with one time (they are one transaction).
#[tokio::test(flavor = "multi_thread")]
async fn a_committed_pair_stamps_both_branches_with_one_time() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let (a, b) = (branch("a"), branch("b"));

    let before = db.clock().await;
    bt.run_pair(&a, &b, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("the pair commits");
    let after = db.clock().await;

    let (va, ta) = db.row(&a).await;
    let (vb, tb) = db.row(&b).await;
    assert_eq!((va, vb), (1, 1));
    assert_eq!(ta, tb, "one transaction, one stamp");
    assert!(
        before <= ta && ta <= after,
        "the pair's stamp {ta} lies within [{before}, {after}]"
    );
    db.finish().await;
}

/// A pair on rows that already exist moves **both** stamps forward, to one time. On fresh rows the default
/// `now()` of the `INSERT` is already the transaction's time, so the test above cannot tell a stamped row from
/// an unstamped one; this one starts from rows stamped by an earlier pair. Added by the ticket author
/// (claude/5d45b017, 2026-10-10) after a throwaway that stamped only one of the pair's rows passed every test.
#[tokio::test(flavor = "multi_thread")]
async fn a_pair_on_existing_rows_moves_both_stamps_forward() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let (a, b) = (branch("a"), branch("b"));

    bt.run_pair(&a, &b, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("a first pair commits");
    let (_, a_first) = db.row(&a).await;
    let (_, b_first) = db.row(&b).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    let before = db.clock().await;
    bt.run_pair(&a, &b, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("a second pair commits");
    let after = db.clock().await;

    let (va, ta) = db.row(&a).await;
    let (vb, tb) = db.row(&b).await;
    assert_eq!((va, vb), (2, 2));
    assert!(
        ta > a_first,
        "a's second stamp {ta} is after its first {a_first}"
    );
    assert!(
        tb > b_first,
        "b's second stamp {tb} is after its first {b_first}"
    );
    assert_eq!(ta, tb, "one transaction, one stamp");
    assert!(
        before <= ta && ta <= after,
        "the pair's stamp {ta} lies within [{before}, {after}]"
    );
    db.finish().await;
}

/// A closure that fails rolls back: neither the version nor the time moves, for `run` and for `run_pair`.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_run_or_pair_moves_neither_version_nor_time() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let (a, b) = (branch("a"), branch("b"));
    bt.run_pair(&a, &b, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("a first pair commits");
    let (va, ta) = db.row(&a).await;
    let (vb, tb) = db.row(&b).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    let err = bt
        .run(&a, |_tx| {
            Box::pin(async { Err::<(), _>(AppError::Domain("refused".to_owned())) })
        })
        .await
        .expect_err("the run's closure refused");
    assert!(matches!(&err, AppError::Domain(s) if s == "refused"));
    let err = bt
        .run_pair(&a, &b, |_tx| {
            Box::pin(async { Err::<(), _>(AppError::Domain("refused".to_owned())) })
        })
        .await
        .expect_err("the pair's closure refused");
    assert!(matches!(&err, AppError::Domain(s) if s == "refused"));

    assert_eq!(
        db.row(&a).await,
        (va, ta),
        "a: neither version nor changed_at moved"
    );
    assert_eq!(
        db.row(&b).await,
        (vb, tb),
        "b: neither version nor changed_at moved"
    );
    db.finish().await;
}
