//! Shared by the xc-auth database tests: a PostgreSQL schema per test and its helpers.
//! Provided with ticket T4b. OR-7.m w2 (#1027): the v2 fixture, the replay and every helper that took
//! or returned `AccountService` / `Employee` / `SignedIn` went with the account module; the database
//! helpers keep their signatures.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(60);

/// v2's sentences (`server.mjs:7293`, `7315`, `7322`, `7338`, `7341`, `7333`, `7350`).
pub const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
pub const LIMIT: &str = "ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที";
pub const PASSWORD_LENGTH: &str = "รหัสผ่านต้องมี 12–128 ตัวอักษร";
pub const EMPLOYEE_DATA: &str = "ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง";
pub const EMPLOYEE_NOT_FOUND: &str = "ไม่พบพนักงาน";
pub const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
pub const MUST_CHANGE: &str = "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน";
/// What v2 throws when there is no usable staff session; in v3 it is `AppError::Unauthenticated`.
pub const AUTH_REQUIRED: &str = "EMPLOYEE_AUTH_REQUIRED";

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// Await `fut`, failing the test when it takes longer than [`MUST_FINISH`].
pub async fn within<T>(what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(MUST_FINISH, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {MUST_FINISH:?}: {what}"),
    }
}

/// A fixed instant, `seconds` after 2026-10-07 03:00:00 UTC — the start of a 15-minute window.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

pub fn at_millis(millis: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(millis)
        .single()
        .expect("a valid instant")
}

/// The sentence a refusal carries, or [`AUTH_REQUIRED`] for `Unauthenticated`.
/// Anything else is not a refusal and fails the test.
pub fn refusal(error: &AppError) -> String {
    match error {
        AppError::Domain(sentence) => sentence.clone(),
        AppError::Unauthenticated => AUTH_REQUIRED.to_owned(),
        other => panic!(
            "expected a refusal (AppError::Domain with v2's sentence, or AppError::Unauthenticated), got {other:?}"
        ),
    }
}

pub struct TestDb {
    pub pool: PgPool,
    pub schema: String,
}

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    ///
    /// `TEST_DATABASE_URL` must name an existing database whose name ends in
    /// `_test`. When it is absent the test FAILS; nothing here skips.
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The account tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`. \
             Use a dedicated test database."
        );

        let schema = unique("t4b");
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
            .max_connections(16)
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

    /// A branch besides `MAIN`. OR-2: an employee's branch must name a `branches` row
    /// (design #550 follow-up, PR 636).
    pub async fn seed_branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING")
            .bind(id)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("insert the branch {id}: {e}"));
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// The audit lines, in no particular order: (id, actor, subject, action, at).
    pub async fn audit(&self) -> Vec<(String, String, String, String, DateTime<Utc>)> {
        sqlx::query_as("SELECT id::text, actor, subject, action, at FROM employee_audit")
            .fetch_all(&self.pool)
            .await
            .expect("read employee_audit")
    }

    pub async fn drop_schema(self) {
        let Self { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

/// How many of each outcome a set of concurrent calls produced.
pub fn tally<T>(results: Vec<Result<T, AppError>>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for result in results {
        let key = match result {
            Ok(_) => "ok".to_owned(),
            Err(e) => refusal(&e),
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}
