//! Ticket #598 (RE-1+RE-3) — the database harness of the rate-engine tests (RE-1).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.1 (tables), §4.6 (states), §4.8 (`PublishedRates`),
//! §5 (the approval core, AP-1, merged), §7 (class diagram), §8 migration 4, §9 RE-1.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose
//! name ends in `_test`. When the variable is absent every test FAILS; none of them skips. Each
//! `TestDb` is a schema of its own with `api/migrations/` applied (migration 4,
//! `20261010120000_rate_engine.sql`, is the implementer's), so SQL in the migration and in the
//! implementation must name tables without a schema prefix.
//!
//! The harness seeds references and policy versions **by SQL into §4.1's tables, with exactly the
//! column names §4.1 lists** (no service writes them in RE-1: references come from RE-2's adapter
//! and policies from RE-3's `POST /api/rate-policies`). Every test seeds at most one reference
//! batch and at most one policy version per branch, so which batch or version is "newest" is
//! never what a test depends on.
//!
//! Signatures this harness assumes (beyond those of AP-1, which are merged):
//! - `xc_rate::application::approval::rate_publication::register(registry: &mut ApprovalRegistry,
//!   pool: PgPool)` registers `RatePublicationSubject` + `RatePublicationPolicy` (§5.2 "the first
//!   subject") under the type name `"RATE_PUBLICATION"`.
//! - `xc_rate::application::draft::rate_draft_service::RateDraftService::new(pool: PgPool,
//!   approvals: Arc<ApprovalService>) -> RateDraftService`.
//! - `xc_rate::application::publication::publish_service::PublishService::new(pool: PgPool)
//!   -> PublishService`.
//! - `xc_rate::infrastructure::publication::published_rates::PgPublishedRates::new(pool: PgPool)`,
//!   which implements `xc_rate::domain::publication::published_rates::PublishedRates` (§4.8).
//! - `xc-rate` depends on `xc-approval` and `xc-auth` (its `application/approval` implements
//!   `ApprovalSubject`/`ApprovalPolicy`, whose methods take `Actor`).
#![allow(dead_code)]

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::application::approval::{rate_feed_publication, rate_publication};
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(30);

pub const MAIN: &str = "MAIN";
pub const OWNER_ID: &str = "owner-1";
pub const RATES_ID: &str = "rates-1";

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

/// Await `fut`, failing the test when it takes longer than `limit`.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

pub fn owner() -> Actor {
    Actor::owner(OWNER_ID)
}

/// A RATES employee of `MAIN`.
pub fn rates() -> Actor {
    Actor::staff(RATES_ID, Role::Rates, BranchId::new(MAIN)).expect("RATES is a staff role")
}

/// Another RATES employee of `MAIN`.
pub fn rates_2() -> Actor {
    Actor::staff("rates-2", Role::Rates, BranchId::new(MAIN)).expect("RATES is a staff role")
}

pub fn main_branch() -> BranchId {
    BranchId::new(MAIN)
}

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known currency code")
}

pub fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a valid rate")
}

pub fn manual(text: &str) -> LineSide {
    LineSide::Manual(rate(text))
}

/// A line whose two sides are AUTO, with no reason.
pub fn auto_line(currency: &str) -> DraftLine {
    DraftLine::new(code(currency), LineSide::Auto, LineSide::Auto, None)
}

/// A line whose two sides are MANUAL, with a reason.
pub fn manual_line(currency: &str, buy: &str, sell: &str) -> DraftLine {
    DraftLine::new(
        code(currency),
        manual(buy),
        manual(sell),
        Some("set by hand for the test".to_owned()),
    )
}

/// The services of the rate engine over one schema, with the rate subject registered in the
/// approval core.
pub struct Engine {
    pub approvals: Arc<ApprovalService>,
    pub drafts: Arc<RateDraftService>,
    pub publisher: Arc<PublishService>,
    pub published: Arc<PgPublishedRates>,
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

/// The connection options of `TEST_DATABASE_URL`, checked to name a `_test` database.
pub fn test_database() -> PgConnectOptions {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             The rate-engine tests need a real PostgreSQL and do not skip. Point it at an \
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
    options
}

/// `api/migrations/`.
pub fn migrations_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

impl TestDb {
    /// A pool confined to a fresh, empty schema; no migration applied yet.
    pub async fn empty() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let options = test_database();
        let schema = unique("re1");
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
        Self { pool, schema }
    }

    /// A pool confined to a fresh schema with every migration of `api/migrations/` applied.
    pub async fn start() -> Self {
        let db = Self::empty().await;
        let dir = migrations_dir();
        Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()))
            .run(&db.pool)
            .await
            .expect("apply api/migrations to the test schema");
        db
    }

    pub fn engine(&self) -> Engine {
        let mut registry = ApprovalRegistry::new();
        rate_publication::register(&mut registry, self.pool.clone());
        rate_feed_publication::register(&mut registry, self.pool.clone());
        let repository: Arc<dyn ApprovalRepository> =
            Arc::new(PgApprovalRepository::new(self.pool.clone()));
        let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(self.pool.clone()));
        let approvals = Arc::new(ApprovalService::new(registry, repository, audit));
        Engine {
            drafts: Arc::new(RateDraftService::new(self.pool.clone(), approvals.clone())),
            publisher: Arc::new(PublishService::new(self.pool.clone())),
            published: Arc::new(PgPublishedRates::new(self.pool.clone())),
            approvals,
        }
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin a transaction")
    }

    pub async fn seed_branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// One reference batch of provider `BOT` fetched at `fetched_at`, with one `BOT` line per
    /// `(currency, source unit, buy, sell)` (§4.1, §4.2: the provider's own unit, values as
    /// received). Answers the batch id.
    pub async fn seed_batch(
        &self,
        fetched_at: DateTime<Utc>,
        lines: &[(&str, &str, Option<&str>, Option<&str>)],
    ) -> String {
        let batch: String = sqlx::query_scalar(
            "INSERT INTO rate_reference_batches \
               (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES (gen_random_uuid(), 'BOT', ($1::timestamptz AT TIME ZONE 'UTC')::date, $1, $2, 'COMPLETE') \
             RETURNING id::text",
        )
        .bind(fetched_at)
        .bind(unique("checksum"))
        .fetch_one(&self.pool)
        .await
        .expect("insert into rate_reference_batches (§4.1)");
        for (currency, unit, buy, sell) in lines {
            sqlx::query(
                "INSERT INTO rate_reference_lines \
                   (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
                 VALUES (gen_random_uuid(), $1::uuid, $2, 'BOT', $3::numeric, $4::numeric, \
                         $5::numeric, '{}'::jsonb)",
            )
            .bind(&batch)
            .bind(*currency)
            .bind(*unit)
            .bind(*buy)
            .bind(*sell)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("insert into rate_reference_lines ({currency}): {e}"));
        }
        batch
    }

    /// Policy version 1 of `branch`, valid since yesterday, entered by the owner (§4.3).
    /// Answers its id (the uuid the in-bound decision names, `policy:<id>`).
    #[allow(clippy::too_many_arguments)]
    pub async fn seed_policy(
        &self,
        branch: &str,
        buy_adjust_pct: &str,
        sell_adjust_pct: &str,
        guards: Value,
        max_reference_age_hours: Option<i32>,
        on_stale: Option<&str>,
        stale_limit_thb: Option<&str>,
    ) -> String {
        sqlx::query_scalar(
            "INSERT INTO rate_policy_versions \
               (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
                approved_by, reason, source_version) \
             VALUES (gen_random_uuid(), $1, 1, $2::numeric, $3::numeric, $4, $5, $6, \
                     $7::numeric, now() - interval '1 day', NULL, $8, 'seeded by the test', NULL) \
             RETURNING id::text",
        )
        .bind(branch)
        .bind(buy_adjust_pct)
        .bind(sell_adjust_pct)
        .bind(guards)
        .bind(max_reference_age_hours)
        .bind(on_stale)
        .bind(stale_limit_thb)
        .bind(OWNER_ID)
        .fetch_one(&self.pool)
        .await
        .expect("insert into rate_policy_versions (§4.1)")
    }

    /// `rate_drafts.status` of one draft.
    pub async fn draft_status(&self, draft_id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM rate_drafts WHERE id::text = $1")
            .bind(draft_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("status of draft {draft_id}: {e}"))
    }

    /// `(id, status, object_version_hash)` of every `RATE_PUBLICATION` request of one draft.
    pub async fn requests_of(&self, draft_id: &str) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT id::text, status, object_version_hash FROM approval_requests \
             WHERE type = 'RATE_PUBLICATION' AND object_id = $1 ORDER BY requested_at, id",
        )
        .bind(draft_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("requests of draft {draft_id}: {e}"))
    }

    /// `(approver_id, outcome, reason)` of every decision of one request.
    pub async fn decisions_of(&self, request_id: &str) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT approver_id, outcome, reason FROM approval_decisions \
             WHERE request_id::text = $1",
        )
        .bind(request_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("decisions of request {request_id}: {e}"))
    }

    /// `rate_drafts.evaluation_hash` of one draft.
    pub async fn evaluation_hash(&self, draft_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT evaluation_hash FROM rate_drafts WHERE id::text = $1")
            .bind(draft_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("evaluation_hash of draft {draft_id}: {e}"))
    }

    /// `branches.current_publication_id` of one branch, as text.
    pub async fn current_publication(&self, branch: &str) -> Option<String> {
        sqlx::query_scalar("SELECT current_publication_id::text FROM branches WHERE id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("current_publication_id of {branch}: {e}"))
    }

    /// Every publication of one branch as `(version, id)`, ascending.
    pub async fn publications(&self, branch: &str) -> Vec<(i32, String)> {
        sqlx::query_as(
            "SELECT version, id::text FROM rate_publications WHERE branch_id = $1 ORDER BY version",
        )
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("publications of {branch}: {e}"))
    }

    /// The physical identity and full content of one publication and its lines: `xmin` moves on
    /// any UPDATE, so equal snapshots mean no row was updated.
    pub async fn publication_snapshot(&self, publication_id: &str) -> (String, String) {
        let head: String = sqlx::query_scalar(
            "SELECT p.xmin::text || ' ' || row_to_json(p)::text \
             FROM rate_publications p WHERE p.id::text = $1",
        )
        .bind(publication_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("publication {publication_id}: {e}"));
        let lines: Option<String> = sqlx::query_scalar(
            "SELECT string_agg(l.xmin::text || ' ' || row_to_json(l)::text, ' | ' ORDER BY l.id) \
             FROM published_rate_lines l WHERE l.publication_id::text = $1",
        )
        .bind(publication_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("lines of publication {publication_id}: {e}"));
        (head, lines.unwrap_or_default())
    }

    /// Drop the schema. Reached only by a test that passed; a failed test leaves its schema
    /// behind for inspection.
    pub async fn finish(self) {
        self.pool.close().await;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(test_database())
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
