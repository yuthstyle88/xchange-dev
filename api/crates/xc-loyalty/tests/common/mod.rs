//! Ticket LY-1 (#1381, design `docs/design/blueprint/membership-points.md` §9.3 row LY-1) — the database
//! harness of the loyalty outbox tests. Written before the implementation by the ticket author
//! (claude/29df2ec5). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test`. When the variable is absent every DB test FAILS; none of them skips. Each `TestDb`
//! is a schema of its own with `api/migrations/` applied, so tests run in parallel.
#![allow(dead_code)]

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;

pub const MUST_FINISH: Duration = Duration::from_secs(30);

/// The organisation every v3 database has (`20261011100000_organizations.sql`).
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// One row of `outbox_events`, read back by the tests.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Row {
    pub id: String,
    pub organization_id: String,
    pub event_type: String,
    pub destination: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub idempotency_key: String,
    pub payload_redacted: Value,
    pub status: String,
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The xc-loyalty tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );

        let schema = unique("ly1");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
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

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        Self { pool, schema }
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin a transaction")
    }

    pub async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM outbox_events")
            .fetch_one(&self.pool)
            .await
            .expect("count outbox_events")
    }

    /// Every row of `outbox_events`, oldest first.
    pub async fn rows(&self) -> Vec<Row> {
        sqlx::query_as(
            "SELECT id::text AS id, organization_id::text AS organization_id, event_type, \
                    destination, aggregate_type, aggregate_id, idempotency_key, \
                    payload_redacted, status \
             FROM outbox_events ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read outbox_events")
    }

    /// The whole stored row as text (every column), for the "no personal data" checks.
    pub async fn row_text(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT to_jsonb(o)::text FROM outbox_events o WHERE id::text = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("read outbox row {id}: {e}"))
    }

    pub async fn finish(self) {
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
