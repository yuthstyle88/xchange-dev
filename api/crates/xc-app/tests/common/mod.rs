//! APP-4 (#994, design `docs/design/blueprint/customer-app.md`) — the database harness and seeds of
//! the planner tests. Written before the implementation by the ticket author's delegate
//! (claude/dell-claude-3, on the lane controller's order). Do not edit, delete or weaken; add tests
//! and helpers in files of your own.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose
//! name ends in `_test`; without it every DB test FAILS, none skips. Each `TestDb` is a schema of
//! its own with `api/migrations/` applied, so APP-1's `20261106100000_customer_app.sql` (the
//! tables and the 7-day settings row of the seeded organisation) is in place. Rate publications are
//! seeded with plain SQL against the rate engine's tables, as `api/tests/quote_common/mod.rs` does,
//! so the preview reads them through `PgPublishedRates` exactly as `QuoteService` does.
#![allow(dead_code, unused_imports)]

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;

use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

pub use xc_app::application::planner::planner_preview::{PlannerEstimate, PlannerPreview};
pub use xc_app::application::planner::planner_service::PlannerService;
pub use xc_app::application::settings::app_settings_service::AppSettingsService;
pub use xc_app::domain::app_error::CustomerAppError;
pub use xc_app::domain::app_session::{AppSessionId, SessionHash};
pub use xc_app::domain::owner::DraftOwner;
pub use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan, DraftSave, PlannerDraft};
pub use xc_app::domain::settings::AppSettings;

pub const MUST_FINISH: Duration = Duration::from_secs(30);
/// The organisation every v3 database has (`20261011100000_organizations.sql`).
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// Seeded by the migrations (`20261007100000_branches_rates.sql`).
pub const MAIN: &str = "MAIN";
/// APP-1's seed: 7 days (owner C4).
pub const SEVEN_DAYS: i64 = 604_800;

static STARTING: Semaphore = Semaphore::const_new(4);

pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn org() -> Uuid {
    Uuid::parse_str(ORG).expect("a uuid")
}

/// A fixed instant plus `seconds`, in whole microseconds, so it round-trips through `timestamptz`.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

pub fn money(text: &str) -> Money {
    Money::parse(text).expect("a THB amount")
}

pub fn foreign(text: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(rust_decimal::Decimal::from_str(text).expect("a decimal"))
}

/// A plan of `quantity` `code` on MAIN.
pub fn plan(code: &str, side: CustomerSide, quantity: &str) -> DraftPlan {
    DraftPlan {
        branch_id: Some(BranchId::new(MAIN)),
        currency: CurrencyCode::parse(code).expect("a currency"),
        customer_side: side,
        amount: DraftAmount::Quantity(foreign(quantity)),
    }
}

pub fn create(plan: DraftPlan) -> DraftSave {
    DraftSave {
        id: None,
        expected_version: None,
        plan,
    }
}

pub fn update(id: Uuid, expected_version: i32, plan: DraftPlan) -> DraftSave {
    DraftSave {
        id: Some(id),
        expected_version: Some(expected_version),
        plan,
    }
}

/// A fresh anonymous session owner.
pub fn session() -> DraftOwner {
    DraftOwner::Session(AppSessionId::generate().hash())
}

pub fn customer(id: &str) -> DraftOwner {
    DraftOwner::Customer(id.to_owned())
}

/// A v3 actor of the seeded organisation holding `role` organisation-wide since `at(-86400)`.
pub fn actor(user_id: &str, role: Role) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(role, None, at(-86_400), None)],
        None,
    )
}

/// One `planner_drafts` row as stored.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DraftRow {
    pub customer_id: Option<String>,
    pub session_hash: Option<String>,
    pub branch_id: Option<String>,
    pub currency_code: String,
    pub customer_side: String,
    pub quantity: Option<String>,
    pub budget_thb: Option<String>,
    pub version: i32,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One published line to seed.
#[derive(Clone)]
pub struct Line {
    pub code: &'static str,
    pub unit_size: &'static str,
    pub buy: Option<&'static str>,
    pub sell: Option<&'static str>,
    /// `Some(policy id)`: an AUTO line whose reference was fetched 30 h ago under a policy whose
    /// references go stale after 24 h (stale at read time). `None`: a MANUAL line (never stale).
    pub stale_under: Option<String>,
}

impl Line {
    pub const fn manual(
        code: &'static str,
        unit_size: &'static str,
        buy: &'static str,
        sell: &'static str,
    ) -> Self {
        Self {
            code,
            unit_size,
            buy: Some(buy),
            sell: Some(sell),
            stale_under: None,
        }
    }

    pub fn stale(mut self, policy: &str) -> Self {
        self.stale_under = Some(policy.to_owned());
        self
    }
}

/// JPY at 23.5 / 24.0 THB per 100 (design §9.1's JPY case), USD at 32.10 / 32.45 per 1.
pub const JPY: Line = Line::manual("JPY", "100", "23.5", "24.0");
pub const USD: Line = Line::manual("USD", "1", "32.10", "32.45");

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

impl TestDb {
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The xc-app tests need a real PostgreSQL and do not \
                 skip, e.g. TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );

        let schema = unique("app4");
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
            .max_connections(6)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin")
    }

    pub async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    pub async fn drafts(&self) -> i64 {
        self.count("SELECT count(*) FROM planner_drafts").await
    }

    /// The stored row of draft `id`, or `None`.
    pub async fn draft_row(&self, id: Uuid) -> Option<DraftRow> {
        sqlx::query_as(
            "SELECT customer_id, session_hash, branch_id, currency_code::text AS currency_code, \
                 customer_side, quantity::text AS quantity, budget_thb::text AS budget_thb, version, \
                 expires_at, created_at, updated_at \
             FROM planner_drafts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .expect("read the draft row")
    }

    /// A LIVE customer of MAIN's organisation (the CI-4 shape).
    pub async fn seed_customer(&self, id: &str) {
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT '{id}', organization_id, '{id}', 'Test Customer', 'LIVE', now() FROM branches WHERE id = '{MAIN}'"
        ))
        .await;
    }

    /// Sets the seeded organisation's draft lifetime (`None` = NULL) on its existing row.
    pub async fn set_lifetime(&self, seconds: Option<i64>) {
        let value = seconds.map_or("NULL".to_owned(), |s| s.to_string());
        self.exec(&format!(
            "UPDATE customer_app_settings SET planner_draft_ttl_seconds = {value} WHERE organization_id = '{ORG}'"
        ))
        .await;
    }

    pub async fn delete_settings(&self) {
        self.exec(&format!(
            "DELETE FROM customer_app_settings WHERE organization_id = '{ORG}'"
        ))
        .await;
    }

    /// (rows, lifetime, updated_by) of the seeded organisation's settings.
    pub async fn settings_row(&self) -> (i64, Option<i32>, Option<String>) {
        let rows = self
            .count(&format!(
                "SELECT count(*) FROM customer_app_settings WHERE organization_id = '{ORG}'"
            ))
            .await;
        let row: Option<(Option<i32>, String)> = sqlx::query_as(
            "SELECT planner_draft_ttl_seconds, updated_by FROM customer_app_settings WHERE organization_id = $1",
        )
        .bind(org())
        .fetch_optional(&self.pool)
        .await
        .expect("read the settings row");
        match row {
            Some((ttl, by)) => (rows, ttl, Some(by)),
            None => (rows, None, None),
        }
    }

    /// Every table the preview must leave alone: `quotes`, `quote_lines`, `bookings`,
    /// `ledger_entries` and `cash_transfers`, with its row count.
    pub async fn untouchable_counts(&self) -> Vec<(String, i64)> {
        let mut tables: Vec<String> = sqlx::query_scalar(
            "SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' \
               AND table_name IN ('quotes', 'quote_lines', 'bookings', 'ledger_entries', 'cash_transfers') \
             ORDER BY table_name",
        )
        .fetch_all(&self.pool)
        .await
        .expect("list the tables");
        assert!(
            tables.iter().any(|t| t == "quotes") && tables.iter().any(|t| t == "bookings"),
            "the schema has `quotes` and `bookings`: {tables:?}"
        );
        assert!(
            tables.iter().any(|t| t == "ledger_entries")
                && tables.iter().any(|t| t == "cash_transfers"),
            "the schema has `ledger_entries` and `cash_transfers`: {tables:?}"
        );
        let mut counts = Vec::new();
        for table in tables.drain(..) {
            let n = self.count(&format!("SELECT count(*) FROM {table}")).await;
            counts.push((table, n));
        }
        counts
    }

    /// A rate policy version of `branch` with `on_stale` (`None` = NULL) and `stale_limit_thb`;
    /// references go stale after 24 h. Answers its id.
    pub async fn policy(
        &self,
        branch: &str,
        on_stale: Option<&str>,
        stale_limit_thb: Option<&str>,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let on_stale = on_stale.map_or("NULL".to_owned(), |s| format!("'{s}'"));
        let limit = stale_limit_thb.map_or("NULL".to_owned(), |s| s.to_owned());
        self.exec(&format!(
            "INSERT INTO rate_policy_versions (id, organization_id, branch_id, version, \
                 buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours, on_stale, \
                 stale_limit_thb, valid_from, valid_to, approved_by, reason, source_version) \
             SELECT '{id}', b.organization_id, b.id, \
                 COALESCE((SELECT max(version) FROM rate_policy_versions WHERE branch_id = b.id), 0) + 1, \
                 0, 0, '{{}}'::jsonb, 24, {on_stale}, {limit}, now() - interval '1 day', NULL, \
                 'test', 'test', NULL \
             FROM branches b WHERE b.id = '{branch}'"
        ))
        .await;
        id
    }

    /// Publishes `lines` on `branch` as its next version and points the branch at it.
    pub async fn publish(&self, branch: &str, lines: &[Line]) {
        let request = Uuid::new_v4().to_string();
        let publication = Uuid::new_v4().to_string();
        let key = unique("pub");
        self.exec(&format!(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                 object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                 status, idempotency_key) \
             SELECT '{request}', organization_id, 'RATE_PUBLICATION', id, '{publication}', 'test', \
                 '1', '{{}}'::jsonb, 'test', now(), 'APPROVED', '{key}' \
             FROM branches WHERE id = '{branch}'"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
                 approved_request_id, draft_id, published_by) \
             SELECT '{publication}', b.organization_id, b.id, \
                 COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
                 now(), '{request}', NULL, 'test' \
             FROM branches b WHERE b.id = '{branch}'"
        ))
        .await;
        let fetched_at = (Utc::now() - chrono::Duration::hours(30)).to_rfc3339();
        for line in lines {
            let id = Uuid::new_v4().to_string();
            let snapshot = match &line.stale_under {
                Some(policy) => json!({
                    "currency": line.code,
                    "buy_mode": "AUTO",
                    "sell_mode": "AUTO",
                    "fetched_at": fetched_at,
                    "source_date": fetched_at[..10],
                    "max_reference_age_hours": 24,
                    "policy_version_id": policy,
                }),
                None => json!({
                    "currency": line.code,
                    "buy_mode": "MANUAL",
                    "sell_mode": "MANUAL",
                    "reason": "set by hand for the test",
                }),
            };
            self.exec(&format!(
                "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                     denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
                 SELECT '{id}', organization_id, '{publication}', '{code}', 'ALL', {unit}, {buy}, \
                     {sell}, '{snapshot}'::jsonb \
                 FROM rate_publications WHERE id = '{publication}'",
                code = line.code,
                unit = line.unit_size,
                buy = line.buy.unwrap_or("NULL"),
                sell = line.sell.unwrap_or("NULL"),
            ))
            .await;
        }
        self.exec(&format!(
            "UPDATE branches SET current_publication_id = '{publication}' WHERE id = '{branch}'"
        ))
        .await;
    }

    pub async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}
