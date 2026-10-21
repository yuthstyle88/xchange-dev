//! Written before the implementation (ticket T6a). Do not edit, delete or weaken.
//!
//! Shared by the customer tests: the database harness and the v2 fixture.
//!
//! The single `pub use` block below is every path of `xc-exchange` the tests name.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied, so tests run in parallel and the migration is
//! exercised from an empty schema every time. SQL in the migration and in the
//! implementation must therefore name tables without a schema prefix.
#![allow(dead_code)]

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde::Deserialize;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

pub use xc_customer::application::customer::service::CustomerService;
pub use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
pub use xc_customer::domain::customer::entities::customer::Customer;
pub use xc_customer::domain::customer::entities::reservation::Reservation;
pub use xc_customer::domain::customer::value_objects::customer_details::CustomerDetails;
pub use xc_kernel::mode::Mode;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(20);

pub const MAIN: &str = "MAIN";

// The sentences v2 answers with in this context.
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const BAD_DATE_FORMAT: &str = "รูปแบบวันที่ไม่ถูกต้อง";
pub const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
pub const BAD_MODE: &str = "กรุณาเลือกประเภทรายการ";
pub const COUNTER_EXHAUSTED: &str = "ตัวนับรหัสไม่ถูกต้อง";
/// Answered by the save of an exchange when the customer id is not that passport's.
pub const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

/// Await `fut`, failing the test when it takes longer than `limit`.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

/// 2026-10-06 12:00:00 UTC plus `seconds`, with microseconds that are not whole
/// milliseconds: what a wall clock hands the service.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

/// The same instant with everything below the millisecond dropped.
pub fn millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .expect("a valid instant")
}

pub fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date written yyyy-mm-dd")
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// A well-formed live customer; `n` makes the passport number.
pub fn person(n: u32) -> ReserveCustomer {
    ReserveCustomer {
        name: format!("Customer {n}"),
        passport_no: format!("P{n:07}"),
        nationality: "THA".to_owned(),
        birth_date: String::new(),
        passport_expiry: String::new(),
        mode: "LIVE".to_owned(),
    }
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The xc-exchange tests need a real PostgreSQL and do not skip. Point it at an \
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

        let schema = unique("t6");
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
            .max_connections(10)
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
        xc_customer::install_pii_keys(std::sync::Arc::new(
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("the test PII key parses"),
        ))
        .expect("one test PII key per process");

        Self { pool, schema }
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    pub fn customers(&self) -> CustomerService {
        CustomerService::new(self.branch_tx())
    }

    /// Add a branch (the table is ticket T5a's).
    pub async fn seed_branch(&self, id: &str, name: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    /// Put a counter where a scenario says v2's numbering stood.
    pub async fn seed_counter(&self, key: &str, value: i64) {
        sqlx::query("INSERT INTO counters (key, value) VALUES ($1, $2)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .expect("insert into counters");
    }

    pub async fn counter(&self, key: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT value FROM counters WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .expect("read counters")
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind for inspection.
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
