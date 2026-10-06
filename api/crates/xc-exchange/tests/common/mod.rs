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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

pub use xc_exchange::application::customer::service::CustomerService;
pub use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
pub use xc_exchange::domain::customer::entities::customer::Customer;
pub use xc_exchange::domain::customer::entities::reservation::Reservation;
pub use xc_exchange::domain::customer::value_objects::customer_details::CustomerDetails;
pub use xc_exchange::domain::customer::value_objects::mode::Mode;

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

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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

/// id, name, passport_no, nationality, birth_date, passport_expiry, mode
type StoredRow = (
    String,
    String,
    String,
    String,
    Option<NaiveDate>,
    Option<NaiveDate>,
    String,
);

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

    /// Every stored customer of a branch, both modes, as the fixture records them.
    pub async fn stored(&self, branch_id: &str) -> Vec<FxStored> {
        let rows: Vec<StoredRow> = sqlx::query_as(
            "SELECT id, name, passport_no, nationality, birth_date, passport_expiry, mode \
                 FROM customers WHERE branch_id = $1",
        )
        .bind(branch_id)
        .fetch_all(&self.pool)
        .await
        .expect("read customers");
        let mut out: Vec<FxStored> = rows
            .into_iter()
            .map(
                |(id, name, passport_no, nationality, birth, expiry, mode)| FxStored {
                    id,
                    name,
                    passport_no,
                    nationality,
                    birth_date: birth.map(|d| d.to_string()),
                    passport_expiry: expiry.map(|d| d.to_string()),
                    mode,
                },
            )
            .collect();
        out.sort();
        out
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

// ── the v2 fixture ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub sentences: Vec<String>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// Where v2's numbering stood before the first step; a key that is absent was never used.
    pub counters: std::collections::BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub op: String,
    pub branch: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub passport_no: String,
    #[serde(default)]
    pub nationality: String,
    #[serde(default)]
    pub birth_date: String,
    #[serde(default)]
    pub passport_expiry: String,
    /// Absent when the screen sent no mode at all.
    #[serde(default)]
    pub mode: String,
    /// What v2 answered: `None` when it accepted the step.
    pub v2_error: Option<String>,
    pub customer_id: Option<String>,
    pub existing: Option<bool>,
    pub stored: Option<Vec<FxStored>>,
    pub listed: Option<Vec<FxListed>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct FxStored {
    pub id: String,
    pub name: String,
    pub passport_no: String,
    pub nationality: String,
    pub birth_date: Option<String>,
    pub passport_expiry: Option<String>,
    pub mode: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct FxListed {
    pub id: String,
    pub name: String,
    pub nationality: String,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v2_customer_cases.json"))
        .expect("tests/fixtures/v2_customer_cases.json parses")
}

/// Play one fixture scenario against a fresh schema and return every place where
/// the service answered differently from v2.
pub async fn play(scenario: &Scenario) -> Vec<String> {
    let db = TestDb::start().await;
    for (key, value) in &scenario.counters {
        db.seed_counter(key, value.parse().expect("a counter is a whole number"))
            .await;
    }
    let customers = db.customers();
    let mut wrong = Vec::new();

    for (i, step) in scenario.steps.iter().enumerate() {
        let here = format!("'{}' step {} ({})", scenario.name, i + 1, step.op);
        match step.op.as_str() {
            "createBranch" => db.seed_branch(&step.branch, &step.name).await,
            "reserve" => {
                let cmd = ReserveCustomer {
                    name: step.name.clone(),
                    passport_no: step.passport_no.clone(),
                    nationality: step.nationality.clone(),
                    birth_date: step.birth_date.clone(),
                    passport_expiry: step.passport_expiry.clone(),
                    mode: step.mode.clone(),
                };
                let got = customers
                    .reserve(&branch(&step.branch), cmd, at(i as i64))
                    .await;
                match (&step.v2_error, &got) {
                    (None, Ok(r)) => {
                        if Some(&r.customer_id) != step.customer_id.as_ref()
                            || Some(r.existing) != step.existing
                        {
                            wrong.push(format!(
                                "{here}: v2 answered ({:?}, existing {:?}), the service answered {r:?}",
                                step.customer_id, step.existing
                            ));
                        }
                    }
                    (Some(want), Err(AppError::Domain(sentence))) if sentence == want => {}
                    (want, got) => wrong.push(format!(
                        "{here}: v2 answered {want:?}, the service answered {got:?}"
                    )),
                }
                if let Some(expected) = &step.stored {
                    let mut expected: Vec<&FxStored> = expected.iter().collect();
                    expected.sort();
                    let got = db.stored(&step.branch).await;
                    if got.iter().collect::<Vec<_>>() != expected {
                        wrong.push(format!(
                            "{here}: stored customers are {got:?}, v2 has {expected:?}"
                        ));
                    }
                }
                if let Some(expected) = &step.listed {
                    match customers.list_live(&branch(&step.branch)).await {
                        Ok(list) => {
                            let got: Vec<FxListed> = list
                                .iter()
                                .map(|c| FxListed {
                                    id: c.id.clone(),
                                    name: c.name.clone(),
                                    nationality: c.nationality.clone(),
                                })
                                .collect();
                            if &got != expected {
                                wrong.push(format!(
                                    "{here}: list_live is {got:?}, v2 lists {expected:?}"
                                ));
                            }
                        }
                        Err(e) => wrong.push(format!("{here}: list_live failed: {e:?}")),
                    }
                }
            }
            other => {
                panic!("{here}: the fixture has an operation this test does not know: {other}")
            }
        }
    }

    if wrong.is_empty() {
        db.finish().await;
    }
    wrong
}
