//! Ticket #669 (TX-1+TX-3) — the database harness shared by `exchange_quotes_migration_test.rs`,
//! `exchange_quote_service_test.rs` and `v3_quotes_route_test.rs`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add helpers in a file of your own.
//!
//! Every test works in a PostgreSQL schema of its own, built from `api/migrations/`. Rate
//! publications are seeded with plain SQL against the RE-1 tables (`rate_publications`,
//! `published_rate_lines`, `branches.current_publication_id`, `rate_policy_versions`), so these
//! tests depend on the rate engine's tables and on `PgPublishedRates::line_in`'s reading of
//! `source_snapshot` (`api/crates/xc-rate/src/infrastructure/publication/published_rates.rs`), not
//! on the draft workflow.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips.

#![allow(dead_code)]

use std::borrow::Cow;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;

/// Design §8 item 1: migration 1's version.
pub const MIGRATION_1: i64 = 20261014100000;
/// The organisation OR-1's migration creates (`20261011100000_organizations.sql`).
pub const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const MUST_FINISH: Duration = Duration::from_secs(30);

static STARTING: Semaphore = Semaphore::const_new(4);

pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             The quote tests need a real PostgreSQL and do not skip. Point it at an existing \
             database whose name ends in `_test`, e.g. \
             TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
        )
    })
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// One published line to seed.
#[derive(Clone)]
pub struct Line {
    pub code: &'static str,
    pub unit_size: &'static str,
    pub buy: Option<&'static str>,
    pub sell: Option<&'static str>,
    /// `Some(policy id)`: an AUTO line whose reference was fetched 30 h ago under a policy whose
    /// references go stale after 24 h (stale at read time, rates §4.8). `None`: a MANUAL line with
    /// no reference (never stale).
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
}

/// A seeded publication: its id, its version, and the id of each line by currency.
pub struct Published {
    pub id: String,
    pub version: i32,
    pub lines: Vec<(String, String)>,
}

impl Published {
    pub fn line(&self, code: &str) -> &str {
        self.lines
            .iter()
            .find(|(c, _)| c == code)
            .map(|(_, id)| id.as_str())
            .unwrap_or_else(|| panic!("{code} was published"))
    }
}

impl TestDb {
    /// An empty schema of its own; no migration applied yet.
    pub async fn empty() -> Self {
        let options =
            PgConnectOptions::from_str(&url()).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("tx669");
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
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Self { pool, schema }
    }

    /// Apply `api/migrations/` up to and including `last` (every one when `None`).
    pub async fn migrate_through(&self, last: Option<i64>) {
        let _slot = STARTING.acquire().await.expect("never closed");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut migrator = Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        assert!(
            migrator.migrations.iter().any(|m| m.version == MIGRATION_1),
            "api/migrations has no migration {MIGRATION_1} (design §8 item 1)"
        );
        if let Some(last) = last {
            let kept: Vec<_> = migrator
                .migrations
                .iter()
                .filter(|m| m.version <= last)
                .cloned()
                .collect();
            migrator.migrations = Cow::Owned(kept);
        }
        migrator
            .run(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("apply api/migrations through {last:?}: {e}"));
    }

    /// Every migration applied.
    pub async fn start() -> Self {
        let db = Self::empty().await;
        db.migrate_through(None).await;
        db
    }

    /// Drop the schema. Reached only by a test that passed.
    pub async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    pub async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// The SQLSTATE `sql` fails with, or `"ok"` when it succeeds.
    pub async fn outcome(&self, sql: &str) -> String {
        match sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
        {
            Ok(_) => "ok".to_owned(),
            Err(e) => match e.as_database_error().and_then(|d| d.code()) {
                Some(code) => code.into_owned(),
                None => panic!("{sql}: not a database error: {e}"),
            },
        }
    }

    /// One text value (`NULL` ⇒ `None`).
    pub async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// `quotes` and `quote_lines` rows, in that order.
    pub async fn quote_rows(&self) -> (i64, i64) {
        (
            self.count("SELECT count(*) FROM quotes").await,
            self.count("SELECT count(*) FROM quote_lines").await,
        )
    }

    /// A second organisation with one branch `branch` (OR-1's tables).
    pub async fn second_organisation(&self, org: &str, code: &str, branch: &str) {
        self.exec(&format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{org}', '{code}', NULL, 'Asia/Bangkok', now())"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('{branch}', '{branch}', '{org}')"
        ))
        .await;
    }

    /// A branch of the first organisation.
    pub async fn branch(&self, branch: &str) {
        self.exec(&format!(
            "INSERT INTO branches (id, name) VALUES ('{branch}', '{branch}')"
        ))
        .await;
    }

    /// A rate policy version of `branch` with `on_stale` (`None` = NULL) and `stale_limit_thb`;
    /// references go stale after 24 h. Answers its id.
    pub async fn policy(
        &self,
        branch: &str,
        on_stale: Option<&str>,
        stale_limit_thb: Option<&str>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
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

    /// Publish `lines` on `branch` as its next version and point the branch at it.
    pub async fn publish(&self, branch: &str, lines: &[Line]) -> Published {
        let request = uuid::Uuid::new_v4().to_string();
        let publication = uuid::Uuid::new_v4().to_string();
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
        let mut ids = Vec::new();
        for line in lines {
            let id = uuid::Uuid::new_v4().to_string();
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
            let buy = line.buy.unwrap_or("NULL");
            let sell = line.sell.unwrap_or("NULL");
            self.exec(&format!(
                "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                     denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
                 SELECT '{id}', organization_id, '{publication}', '{code}', 'ALL', {unit}, {buy}, \
                     {sell}, '{snapshot}'::jsonb \
                 FROM rate_publications WHERE id = '{publication}'",
                code = line.code,
                unit = line.unit_size,
            ))
            .await;
            ids.push((line.code.to_owned(), id));
        }
        self.exec(&format!(
            "UPDATE branches SET current_publication_id = '{publication}' WHERE id = '{branch}'"
        ))
        .await;
        let version = self
            .count(&format!(
                "SELECT version::bigint FROM rate_publications WHERE id = '{publication}'"
            ))
            .await as i32;
        Published {
            id: publication,
            version,
            lines: ids,
        }
    }

    /// The branch's current publication id.
    pub async fn current_publication(&self, branch: &str) -> String {
        self.text(&format!(
            "SELECT current_publication_id::text FROM branches WHERE id = '{branch}'"
        ))
        .await
        .unwrap_or_else(|| panic!("{branch} has a current publication"))
    }

    /// A LIVE customer of `branch`'s organisation (CI-4 #651: a customer belongs to the organisation).
    pub async fn customer(&self, id: &str, branch: &str) {
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT '{id}', organization_id, '{id}', 'Test Customer', 'LIVE', now() FROM branches WHERE id = '{branch}'"
        ))
        .await;
    }
}

/// `now` cut to whole seconds, so it round-trips through `timestamptz` unchanged.
pub fn whole_second_now() -> DateTime<Utc> {
    DateTime::from_timestamp(Utc::now().timestamp(), 0).expect("a valid instant")
}

/// JPY at 23.5 / 24.0 THB per 100 (design §9.1's example), USD at 32.10 / 32.45 per 1.
pub const JPY: Line = Line::manual("JPY", "100", "23.5", "24.0");
pub const USD: Line = Line::manual("USD", "1", "32.10", "32.45");
