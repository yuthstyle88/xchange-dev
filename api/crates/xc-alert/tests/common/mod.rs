//! AL-2 (#1002, rate alerts RA-2 of #561) — the database harness of the `xc-alert` tests.
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rate-alerts.md` §4.1 (tables), §4.3 (rules), §9.1 rows "RA-2".
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose name ends in
//! `_test`. When the variable is absent every database test FAILS; none of them skips. Each `TestDb` is a schema of
//! its own with `api/migrations/` applied, so the tests run in parallel. SQL in the implementation must therefore
//! name tables without a schema prefix.
//!
//! The world every test starts from (`TestDb::world`):
//! - organisation `ORG` (seeded by the organisations migration), branch `MAIN` (and `NORTH` for scope tests);
//! - customers `CX-1` and `CX-2` of `ORG`, and an EXECUTIVE user `EXEC`;
//! - publication v1 of `MAIN`, current, fresh, AUTO on both sides:
//!   USD unit 1 buy 33.50 sell 34.00 · JPY unit 100 buy 21.00 sell 22.00 · KRW unit 1000 buy 24.00 sell 26.00.
//! - **no** `rate_alert_settings` row; tests that create alerts call `settings(..)` first.
//!
//! Signatures this harness assumes (the ticket's §7, design §5):
//! - `RateAlertService::new(Arc<dyn RateAlertRepository>, Arc<dyn CurrencyRegister>, Arc<dyn PublishedRates>)`,
//!   `PgRateAlertRepository::new(PgPool)`; the register and the published rates are `xc-rate`'s Pg adapters.
//! - every service method takes the caller's transaction first; the caller commits.
#![allow(dead_code, unused_imports)] // each test binary uses part of the harness

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Row, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::domain::currency::currency_register::CurrencyRegister;
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::infrastructure::currency::currency_register::PgCurrencyRegister;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

pub use xc_alert::application::rate_alert_service::{
    CreateRateAlert, RateAlertError, RateAlertService,
};
pub use xc_alert::domain::CustomerSide;
pub use xc_alert::domain::condition::AlertCondition;
pub use xc_alert::domain::rate_alert::{RateAlert, RateAlertId};
pub use xc_alert::domain::relation::Relation;
pub use xc_alert::domain::repository::RateAlertRepository;
pub use xc_alert::domain::settings::{AlertSettings, SaveAlertSettings};
pub use xc_alert::domain::source_policy::ObservePolicy;
pub use xc_alert::infrastructure::repository::PgRateAlertRepository;

pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const EXEC: &str = "c1000000-0000-0000-0000-000000000001";
pub const MAIN: &str = "MAIN";
pub const NORTH: &str = "NORTH";
pub const CX1: &str = "CX-1";
pub const CX2: &str = "CX-2";

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(20);
/// How long a transaction is held open to let a concurrent one run into it.
pub const HOLD: Duration = Duration::from_millis(300);

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

/// An idempotency key, different on every call.
pub fn key() -> String {
    unique("key")
}

pub fn org() -> Uuid {
    Uuid::parse_str(ORG).expect("a uuid")
}

pub fn exec() -> Uuid {
    Uuid::parse_str(EXEC).expect("a uuid")
}

pub fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

pub fn currency(code: &str) -> CurrencyCode {
    CurrencyCode::parse(code).expect("a register code")
}

/// A fixed instant, so expiry arithmetic is exact.
pub fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-10T03:00:00Z")
        .expect("rfc3339")
        .with_timezone(&Utc)
}

/// A fresh AUTO snapshot (no `fetched_at`, so `Freshness::of_side` says not stale).
pub fn fresh_auto() -> Value {
    json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" })
}

/// A stale snapshot: fetched in 2020, at most 24 hours old allowed.
pub fn stale_auto() -> Value {
    json!({
        "buy_mode": "AUTO",
        "sell_mode": "AUTO",
        "fetched_at": "2020-01-01T00:00:00Z",
        "max_reference_age_hours": 24
    })
}

/// Await `fut`, failing the test when it takes longer than `limit`.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

/// The refusal's `(code, reason)`. Panics on success.
pub fn refusal<T: std::fmt::Debug>(
    result: Result<T, RateAlertError>,
) -> (&'static str, Option<&'static str>) {
    match result {
        Err(e) => (e.code(), e.reason()),
        Ok(value) => panic!("expected a refusal, got Ok({value:?})"),
    }
}

/// A create input for `MAIN` with no `expiresAt`.
pub fn input(
    code: &str,
    side: CustomerSide,
    condition: AlertCondition,
    target: &str,
) -> CreateRateAlert {
    CreateRateAlert {
        branch: BranchId::new(MAIN),
        currency: currency(code),
        customer_side: side,
        condition,
        target: dec(target),
        expires_at: None,
    }
}

/// One line of a publication: `(currency, unit_size, buy, sell, source_snapshot)`.
pub struct Line {
    pub currency: &'static str,
    pub unit: &'static str,
    pub buy: Option<&'static str>,
    pub sell: Option<&'static str>,
    pub snapshot: Value,
}

pub fn line(
    currency: &'static str,
    unit: &'static str,
    buy: &'static str,
    sell: &'static str,
) -> Line {
    Line {
        currency,
        unit,
        buy: Some(buy),
        sell: Some(sell),
        snapshot: fresh_auto(),
    }
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied (no world yet).
    pub async fn empty() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The xc-alert tests need a real PostgreSQL and do not skip. Point it at an existing database \
                 whose name ends in `_test`, e.g.\n\
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

        let schema = unique("al2");
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

    /// `empty()` plus the world described in the module header.
    pub async fn world() -> Self {
        let db = Self::empty().await;
        db.exec("INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN'), ('NORTH', 'NORTH') ON CONFLICT (id) DO NOTHING")
            .await;
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('{CX1}', '{ORG}', '{CX1}', 'Somchai', 'LIVE', now()), \
             ('{CX2}', '{ORG}', '{CX2}', 'Somsri', 'LIVE', now())"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{EXEC}', '{ORG}', 'Exec', 'ACTIVE', 'test')"
        ))
        .await;
        db.publish(
            MAIN,
            1,
            vec![
                line("USD", "1", "33.50", "34.00"),
                line("JPY", "100", "21.00", "22.00"),
                line("KRW", "1000", "24.00", "26.00"),
            ],
        )
        .await;
        db
    }

    /// The core under test, on `xc-rate`'s Pg register and published rates.
    pub fn service(&self) -> Arc<RateAlertService> {
        let repository: Arc<dyn RateAlertRepository> =
            Arc::new(PgRateAlertRepository::new(self.pool.clone()));
        let register: Arc<dyn CurrencyRegister> =
            Arc::new(PgCurrencyRegister::new(self.pool.clone()));
        let published: Arc<dyn PublishedRates> = Arc::new(PgPublishedRates::new(self.pool.clone()));
        Arc::new(RateAlertService::new(repository, register, published))
    }

    pub fn repository(&self) -> PgRateAlertRepository {
        PgRateAlertRepository::new(self.pool.clone())
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin a transaction")
    }

    pub async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// Publication `version` of `branch` with `lines`, made the branch's current one. Returns the publication id.
    pub async fn publish(&self, branch: &str, version: i32, lines: Vec<Line>) -> Uuid {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
             RETURNING id",
        )
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("an approved request for the publication");
        let publication: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, $2, now(), $3, 'test') RETURNING id",
        )
        .bind(branch)
        .bind(version)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("the publication");
        for l in lines {
            sqlx::query(
                "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
                 source_snapshot) VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6)",
            )
            .bind(publication)
            .bind(l.currency)
            .bind(dec(l.unit))
            .bind(l.buy.map(dec))
            .bind(l.sell.map(dec))
            .bind(&l.snapshot)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("line {} of v{version}: {e}", l.currency));
        }
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("move the current publication");
        publication
    }

    /// The `rate_alert_settings` row of `ORG`, written directly (not through the service).
    pub async fn settings(&self, cooldown: i32, max: i32, lifetime: Option<i32>, policy: &str) {
        sqlx::query(
            "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, \
             default_lifetime_days, observe_policy, updated_by, updated_at) VALUES ($1, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (organization_id) DO UPDATE SET default_cooldown_seconds = EXCLUDED.default_cooldown_seconds, \
             max_active_per_customer = EXCLUDED.max_active_per_customer, \
             default_lifetime_days = EXCLUDED.default_lifetime_days, observe_policy = EXCLUDED.observe_policy",
        )
        .bind(org())
        .bind(cooldown)
        .bind(max)
        .bind(lifetime)
        .bind(policy)
        .bind(exec())
        .execute(&self.pool)
        .await
        .expect("write the settings row");
    }

    /// The owner's 2026-10-10 values (design §0): 24 h, 10, 30 days, `ALL_FRESH`.
    pub async fn owners_settings(&self) {
        self.settings(86400, 10, Some(30), "ALL_FRESH").await;
    }

    /// Create through the service in a transaction of its own; commit on success, roll back on a refusal.
    pub async fn create(
        &self,
        svc: &RateAlertService,
        customer: &str,
        input: &CreateRateAlert,
        key: &str,
    ) -> Result<RateAlert, RateAlertError> {
        let mut tx = self.begin().await;
        let result = svc.create(&mut tx, customer, input, key, t0()).await;
        settle(tx, result).await
    }

    /// A row of `rate_alerts` straight from the table.
    pub async fn row(&self, id: RateAlertId) -> AlertRow {
        let r = sqlx::query(
            "SELECT customer_id, branch_id, currency_code, shop_side, unit_size, condition, target, enabled, \
             cooldown_seconds, expires_at, relation, last_publication_version, episode_no, deleted_at, version, \
             idempotency_key, organization_id \
             FROM rate_alerts WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the rate_alerts row {id}: {e}"));
        AlertRow {
            customer_id: r.get("customer_id"),
            branch_id: r.get("branch_id"),
            currency_code: r.get("currency_code"),
            shop_side: r.get("shop_side"),
            unit_size: r.get("unit_size"),
            condition: r.get("condition"),
            target: r.get("target"),
            enabled: r.get("enabled"),
            cooldown_seconds: r.get("cooldown_seconds"),
            expires_at: r.get("expires_at"),
            relation: r.get("relation"),
            last_publication_version: r.get("last_publication_version"),
            episode_no: r.get("episode_no"),
            deleted_at: r.get("deleted_at"),
            version: r.get("version"),
            idempotency_key: r.get("idempotency_key"),
            organization_id: r.get("organization_id"),
        }
    }

    /// Insert an alert directly. `extra` is a list of `column = value` SQL overrides on top of a plain alert of
    /// `customer` on `MAIN` USD SELL ABOVE 34 (enabled, expiring in 2099, never observed).
    pub async fn insert_alert(
        &self,
        customer: &str,
        branch: &str,
        extra: &[(&str, &str)],
    ) -> RateAlertId {
        let id = Uuid::new_v4();
        let mut columns = vec![
            ("id", format!("'{id}'")),
            ("organization_id", format!("'{ORG}'")),
            ("customer_id", format!("'{customer}'")),
            ("branch_id", format!("'{branch}'")),
            ("currency_code", "'USD'".to_owned()),
            ("shop_side", "'SELL'".to_owned()),
            ("unit_size", "1".to_owned()),
            ("condition", "'ABOVE'".to_owned()),
            ("target", "34.00000000".to_owned()),
            ("cooldown_seconds", "86400".to_owned()),
            ("expires_at", "'2099-01-01T00:00:00Z'".to_owned()),
            ("idempotency_key", format!("'{}'", key())),
        ];
        for (column, value) in extra {
            match columns.iter_mut().find(|(c, _)| c == column) {
                Some(slot) => slot.1 = (*value).to_owned(),
                None => columns.push((column, (*value).to_owned())),
            }
        }
        let names: Vec<&str> = columns.iter().map(|(c, _)| *c).collect();
        let values: Vec<String> = columns.iter().map(|(_, v)| v.clone()).collect();
        self.exec(&format!(
            "INSERT INTO rate_alerts ({}) VALUES ({})",
            names.join(", "),
            values.join(", ")
        ))
        .await;
        RateAlertId::new(id)
    }

    pub async fn alerts_of(&self, customer: &str) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM rate_alerts WHERE customer_id = '{customer}'"
        ))
        .await
    }

    pub async fn events(&self) -> i64 {
        self.count("SELECT count(*) FROM rate_alert_events").await
    }

    /// Drop the schema. Reached only by a test that passed; a failed test leaves its schema for inspection.
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

/// Commit on success, roll back on a refusal.
pub async fn settle<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, RateAlertError>,
) -> Result<T, RateAlertError> {
    match result {
        Ok(value) => {
            tx.commit().await.expect("commit");
            Ok(value)
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e)
        }
    }
}

/// A `rate_alerts` row as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertRow {
    pub customer_id: String,
    pub branch_id: String,
    pub currency_code: String,
    pub shop_side: String,
    pub unit_size: Decimal,
    pub condition: String,
    pub target: Decimal,
    pub enabled: bool,
    pub cooldown_seconds: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub relation: String,
    pub last_publication_version: Option<i32>,
    pub episode_no: i32,
    pub deleted_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub idempotency_key: String,
    pub organization_id: Uuid,
}
