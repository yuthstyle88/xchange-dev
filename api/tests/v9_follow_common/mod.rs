//! The harness of the V.9 FOLLOW / visibility / display / board tests (pricing-tiers.md §15.5, §15.7, §15.8,
//! §15.8a, §15.9): `v9_pricing_follow_test.rs`, `v9_pricing_public_test.rs`, `v9_pricing_display_test.rs`,
//! `v9_pricing_board_test.rs` and `v9_pricing_kiosk_test.rs`.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own. Copied in the style of `pricing_common/mod.rs` (which is not
//! edited) plus the seeds these tests need: market batches with an explicit `fetched_at`, `market_feed_status`
//! rows, policy versions, publications straight into the tables and through the draft path, and a
//! `MarketPricingLoop` built from the real services.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Every test fails
//! without it; none skips. Each `Room` is a schema of its own with `api/migrations/` applied.
//!
//! `H:` is the owner's handoff `PRICING-DESIGN.md` (pricing-tiers.md §15, kept out of the repo).
#![allow(dead_code)]

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::market::market_refresh::MarketRefresh;
use xc_rate::application::pricing::branch_push::BranchPush;
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::rate_feed::market_pricing_loop::MarketPricingLoop;
use xchange::presentation::http::v3::commands::make_approval_service;
use xchange::presentation::http::v3::published_rates::published_rates_data;
use xchange::presentation::pages::{Callers, SessionReader};

#[path = "../common/mod.rs"]
mod common;

pub const MUST_FINISH: Duration = Duration::from_secs(30);
pub const AS: &str = "x-test-as";
pub const HOST: &str = "xc.test";
pub const SAME_ORIGIN: &str = "http://xc.test";
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// The seeded organisation's code (`20261011100000_organizations.sql`), for the public route.
pub const XC: &str = "XC";
pub const MAIN: &str = "MAIN";

pub const MODE: &str = "/api/pricing/mode";
pub const BRANCH_POLICY: &str = "/api/pricing/branch-policy";
pub const DISPLAY: &str = "/api/pricing/display";
pub const MARKET_REFRESH: &str = "/api/pricing/market-refresh";
pub const APPROVE: &str = "/api/pricing/approve";

/// PV-1 (`H:` §4.4, `pricing_suggestion_test.rs:79-93`): TT USD 33.20 / 33.90, d = 2, V.9's default betterment.
pub const PV1_TT: (&str, &str) = ("33.20", "33.90");
pub const PV1_TIERS: [(&str, &str); 4] = [
    ("33.23", "33.87"),
    ("33.24", "33.86"),
    ("33.26", "33.84"),
    ("33.29", "33.80"),
];
/// The BoT base the suggestion converts margins against (`pricing_suggestion_test.rs:51-63`: no base ⇒
/// `NoAnchor`), so a FOLLOW plan is blocked for no reason but the one a test pins.
pub const USD_BOT: (&str, &str) = ("33", "34");

pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap_or_else(|e| panic!("invalid decimal '{text}': {e}"))
}

/// A JSON decimal, a string or a number (so `"1.5"` and `"1.5000"` compare equal).
pub fn dec_of(v: &Value) -> Decimal {
    match v {
        Value::String(s) => dec(s),
        Value::Number(n) => dec(&n.to_string()),
        other => panic!("a decimal, got {other}"),
    }
}

pub fn pairs(list: &[(&str, &str)]) -> Vec<(Decimal, Decimal)> {
    list.iter().map(|(b, s)| (dec(b), dec(s))).collect()
}

/// `now` cut to whole seconds, so it round-trips through `timestamptz` unchanged.
pub fn whole_second_now() -> DateTime<Utc> {
    DateTime::from_timestamp(Utc::now().timestamp(), 0).expect("a valid instant")
}

pub fn minutes(n: i64) -> chrono::Duration {
    chrono::Duration::minutes(n)
}

pub fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD")
}

// ------------------------------------------------------------ database

pub struct Room {
    pub pool: PgPool,
    schema: String,
}

impl Room {
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("v9follow");
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
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    pub async fn finish(self) {
        let Room { pool, schema } = self;
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

    pub async fn count(&self, sql_from_where: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT count(*) FROM {sql_from_where}"
        )))
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("count {sql_from_where}: {e}"))
    }

    /// An active branch of the seeded organisation whose code is its id.
    pub async fn create_branch(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO branches (id, name, code, organization_id) VALUES ($1, $1, $1, $2::uuid) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("create branch");
    }

    /// A current policy version of `branch`: adjust 0 / 0, value tiers OFF, references stale after 24 h, and a
    /// ±5 % guard on both sides for every currency these tests price. Answers its id.
    pub async fn policy(&self, branch: &str) -> Uuid {
        let g = json!({ "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
                        "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } });
        let mut guards = serde_json::Map::new();
        for code in ["USD", "EUR", "GBP", "JPY", "CNY", "AUD", "KHR"] {
            guards.insert(code.to_owned(), g.clone());
        }
        sqlx::query_scalar(
            "INSERT INTO rate_policy_versions \
               (id, organization_id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                max_reference_age_hours, valid_from, valid_to, approved_by, reason, value_tiers_on) \
             SELECT gen_random_uuid(), b.organization_id, b.id, \
                 COALESCE((SELECT max(version) FROM rate_policy_versions WHERE branch_id = b.id), 0) + 1, \
                 0, 0, $2, 24, now() - interval '1 day', NULL, 'owner-1', 'seeded', false \
             FROM branches b WHERE b.id = $1 RETURNING id",
        )
        .bind(branch)
        .bind(Value::Object(guards))
        .fetch_one(&self.pool)
        .await
        .expect("insert rate_policy_versions")
    }

    /// The current policy version id of `branch`.
    pub async fn current_policy(&self, branch: &str) -> Uuid {
        sqlx::query_scalar(
            "SELECT id FROM rate_policy_versions WHERE branch_id = $1 AND valid_to IS NULL \
             ORDER BY version DESC LIMIT 1",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .expect("a current policy version")
    }

    /// One reference batch of `provider` / `reference_type` fetched at `fetched_at`, one line per
    /// (currency, buy, sell), unit 1, `raw.decimals = decimals` (the TT `d` the suggestion reads). Answers its id.
    pub async fn reference_batch(
        &self,
        provider: &str,
        reference_type: &str,
        fetched_at: DateTime<Utc>,
        lines: &[(&str, &str, &str)],
        decimals: Option<u32>,
    ) -> Uuid {
        let batch: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES (gen_random_uuid(), $1, ($2::timestamptz AT TIME ZONE 'UTC')::date, $2, $3, 'COMPLETE') \
             RETURNING id",
        )
        .bind(provider)
        .bind(fetched_at)
        .bind(unique("chk"))
        .fetch_one(&self.pool)
        .await
        .expect("insert rate_reference_batches");
        for (currency, buy, sell) in lines {
            let raw = json!({ "code": currency, "buy": buy, "sell": sell, "decimals": decimals });
            sqlx::query(
                "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
                 VALUES (gen_random_uuid(), $1, $2, $3, 1, $4::numeric, $5::numeric, $6)",
            )
            .bind(batch)
            .bind(*currency)
            .bind(reference_type)
            .bind(*buy)
            .bind(*sell)
            .bind(raw)
            .execute(&self.pool)
            .await
            .expect("insert rate_reference_lines");
        }
        batch
    }

    /// A TT batch fetched at `fetched_at` (d = 2) and `market_feed_status` saying that fetch was TT's last
    /// attempt, with no error: the source is current until 30 minutes after `fetched_at` (`H:` §6).
    pub async fn tt(&self, fetched_at: DateTime<Utc>, lines: &[(&str, &str, &str)]) -> Uuid {
        let id = self
            .reference_batch("TT", "MARKET_TT", fetched_at, lines, Some(2))
            .await;
        self.feed_status("TT", fetched_at, None).await;
        id
    }

    /// A Super Rich batch fetched at `fetched_at`, recorded as SR's last attempt.
    pub async fn sr(&self, fetched_at: DateTime<Utc>, lines: &[(&str, &str, &str)]) -> Uuid {
        let id = self
            .reference_batch("SR", "MARKET_SR", fetched_at, lines, Some(2))
            .await;
        self.feed_status("SR", fetched_at, None).await;
        id
    }

    /// A BoT batch fetched an hour ago.
    pub async fn bot(&self, lines: &[(&str, &str, &str)]) -> Uuid {
        let at = whole_second_now() - minutes(60);
        self.reference_batch("BOT", "BOT", at, lines, None).await
    }

    /// Upserts `market_feed_status` for `source` (`TT` · `SR`).
    pub async fn feed_status(&self, source: &str, at: DateTime<Utc>, error: Option<&str>) {
        sqlx::query(
            "INSERT INTO market_feed_status (source, last_attempt_at, last_error) VALUES ($1, $2, $3) \
             ON CONFLICT (source) DO UPDATE SET last_attempt_at = excluded.last_attempt_at, \
                 last_error = excluded.last_error",
        )
        .bind(source)
        .bind(at)
        .bind(error)
        .execute(&self.pool)
        .await
        .expect("upsert market_feed_status");
    }

    /// Moves batch `id`'s `fetched_at` (and TT's last attempt with it) to `at`: the same source state as if the
    /// clock had moved on with no new fetch.
    pub async fn tt_fetched_at(&self, id: Uuid, at: DateTime<Utc>) {
        sqlx::query("UPDATE rate_reference_batches SET fetched_at = $2 WHERE id = $1")
            .bind(id)
            .bind(at)
            .execute(&self.pool)
            .await
            .expect("move fetched_at");
        self.feed_status("TT", at, None).await;
    }

    /// Publishes MANUAL lines `(code, unit, buy, sell)` on `branch` as its next version, straight into the
    /// tables (the shape of `quote_common::TestDb::publish`). Answers the publication id.
    pub async fn publish_manual(&self, branch: &str, lines: &[(&str, &str, &str, &str)]) -> Uuid {
        let request = Uuid::new_v4();
        let publication = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                 object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                 status, idempotency_key) \
             SELECT $1, organization_id, 'RATE_PUBLICATION', id, $2, 'test', '1', '{}'::jsonb, 'test', now(), \
                 'APPROVED', $4 \
             FROM branches WHERE id = $3",
        )
        .bind(request)
        .bind(publication)
        .bind(branch)
        .bind(unique("pub"))
        .execute(&self.pool)
        .await
        .expect("insert approval_requests");
        sqlx::query(
            "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
                 approved_request_id, draft_id, published_by) \
             SELECT $1, b.organization_id, b.id, \
                 COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
                 now(), $2, NULL, 'test' \
             FROM branches b WHERE b.id = $3",
        )
        .bind(publication)
        .bind(request)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("insert rate_publications");
        for (code, unit, buy, sell) in lines {
            let snapshot = json!({
                "currency": code,
                "buy_mode": "MANUAL",
                "sell_mode": "MANUAL",
                "reason": "set by hand for the test",
            });
            sqlx::query(
                "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                     denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
                 SELECT gen_random_uuid(), organization_id, $1, $2, 'ALL', $3::numeric, $4::numeric, \
                     $5::numeric, $6 \
                 FROM rate_publications WHERE id = $1",
            )
            .bind(publication)
            .bind(*code)
            .bind(*unit)
            .bind(*buy)
            .bind(*sell)
            .bind(snapshot)
            .execute(&self.pool)
            .await
            .expect("insert published_rate_lines");
        }
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("point the branch at the publication");
        publication
    }

    /// The owner publishes one all-`AUTO` line per code on `branch` through the draft path, as today
    /// (`rate_feed_loop_test.rs::owner_publishes_auto_usd`), so a later `RateDraftService::copy` finds its draft.
    /// Needs the branch's policy and a BoT line per code.
    pub async fn owner_publishes_auto(&self, branch: &str, codes: &[&str]) {
        let b = BranchId::new(branch);
        let svc = drafts(&self.pool);
        let lines = codes
            .iter()
            .map(|code| {
                DraftLine::new(
                    CurrencyCode::parse(code).expect("a currency"),
                    LineSide::Auto,
                    LineSide::Auto,
                    None,
                )
            })
            .collect();
        let who = owner_actor();
        let mut tx = self.pool.begin().await.expect("begin");
        let draft = svc
            .create(&mut tx, &b, &who, lines)
            .await
            .unwrap_or_else(|e| panic!("create: {}", e.code()));
        svc.submit(&mut tx, &b, &who, &draft.id, &unique("owner-key"))
            .await
            .unwrap_or_else(|e| panic!("submit: {}", e.code()));
        let expected: Option<i32> = sqlx::query_scalar(
            "SELECT p.version FROM branches b JOIN rate_publications p \
               ON p.id = b.current_publication_id WHERE b.id = $1",
        )
        .bind(branch)
        .fetch_optional(&mut *tx)
        .await
        .expect("read the current version");
        PublishService::new(self.pool.clone())
            .publish(&mut tx, &b, &who, &draft.id, expected)
            .await
            .unwrap_or_else(|e| panic!("publish: {}", e.code()));
        tx.commit().await.expect("commit");
    }

    /// `branch`'s current publication id, if any.
    pub async fn current_publication(&self, branch: &str) -> Option<Uuid> {
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("read the branch")
    }

    /// `(buy, sell)` of every published line of `code` in `branch`'s current publication, tier 1 first
    /// (`tier_lower_exclusive_thb` NULL first, then ascending).
    pub async fn published_tiers(&self, branch: &str, code: &str) -> Vec<(Decimal, Decimal)> {
        let rows: Vec<(Option<Decimal>, Option<Decimal>)> = sqlx::query_as(
            "SELECT l.buy_rate, l.sell_rate FROM published_rate_lines l \
             JOIN branches b ON b.current_publication_id = l.publication_id \
             WHERE b.id = $1 AND trim(l.currency_code) = $2 \
             ORDER BY l.tier_lower_exclusive_thb NULLS FIRST",
        )
        .bind(branch)
        .bind(code)
        .fetch_all(&self.pool)
        .await
        .expect("published tiers");
        rows.into_iter()
            .map(|(b, s)| (b.expect("a buy rate"), s.expect("a sell rate")))
            .collect()
    }

    /// Every `pricing_tier_sets` row of (branch, code), oldest first.
    pub async fn tier_sets(&self, branch: &str, code: &str) -> Vec<TierSetRow> {
        let rows: Vec<(Uuid, String, String, Option<Uuid>, Value)> = sqlx::query_as(
            "SELECT id, request_id, source, base_tier_set_id, tiers FROM pricing_tier_sets \
             WHERE branch_id = $1 AND trim(currency_code) = $2 ORDER BY approved_at, id",
        )
        .bind(branch)
        .bind(code)
        .fetch_all(&self.pool)
        .await
        .expect("pricing_tier_sets");
        rows.into_iter()
            .map(|(id, request_id, source, base, tiers)| TierSetRow {
                id,
                request_id,
                source,
                base,
                tiers: tiers
                    .as_array()
                    .expect("tiers is an array")
                    .iter()
                    .map(|t| (dec_of(&t["buy"]), dec_of(&t["sell"])))
                    .collect(),
            })
            .collect()
    }

    /// MAIN's `pricing_follow_results` row for `code`.
    pub async fn follow_result(&self, code: &str) -> Option<FollowRow> {
        let row: Option<FollowRowTuple> = sqlx::query_as(
            "SELECT status, reasons, anchor_batch_id, tier_set_id FROM pricing_follow_results \
             WHERE trim(currency_code) = $1",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await
        .expect("pricing_follow_results");
        row.map(
            |(status, reasons, anchor_batch_id, tier_set_id)| FollowRow {
                status,
                reasons,
                anchor_batch_id,
                tier_set_id,
            },
        )
    }

    /// `rate_publications` rows of `branch`.
    pub async fn publications(&self, branch: &str) -> i64 {
        self.count(&format!("rate_publications WHERE branch_id = '{branch}'"))
            .await
    }

    /// A kiosk switch (`kiosk_currencies`).
    pub async fn kiosk_switch(&self, branch: &str, code: &str, enabled: bool) {
        sqlx::query(
            "INSERT INTO branch_currencies (branch_id, currency_code, display, updated_by) \
             VALUES ($1, $2, CASE WHEN $3 THEN 'SHOW' ELSE 'HIDE' END, 'test') \
             ON CONFLICT (branch_id, currency_code, group_code) DO UPDATE \
             SET display = excluded.display, updated_by = excluded.updated_by",
        )
        .bind(branch)
        .bind(code)
        .bind(enabled)
        .execute(&self.pool)
        .await
        .expect("switch a kiosk currency");
    }

    /// A `rate_display_settings` row of `branch`, straight into the table.
    pub async fn display_row(
        &self,
        branch: &str,
        tiers: &[i16],
        featured: &[&str],
        highlight: bool,
        all_market: bool,
    ) {
        let featured: Vec<String> = featured.iter().map(|s| (*s).to_owned()).collect();
        sqlx::query(
            "INSERT INTO rate_display_settings (branch_id, tiers, featured, highlight, all_market, updated_by, updated_at) \
             VALUES ($1, $2, $3::char(3)[], $4, $5, 'owner-1', now()) \
             ON CONFLICT (branch_id) DO UPDATE SET tiers = excluded.tiers, featured = excluded.featured, \
                 highlight = excluded.highlight, all_market = excluded.all_market, updated_at = now()",
        )
        .bind(branch)
        .bind(tiers)
        .bind(featured)
        .bind(highlight)
        .bind(all_market)
        .execute(&self.pool)
        .await
        .expect("insert rate_display_settings");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierSetRow {
    pub id: Uuid,
    pub request_id: String,
    pub source: String,
    pub base: Option<Uuid>,
    pub tiers: Vec<(Decimal, Decimal)>,
}

type FollowRowTuple = (String, Vec<String>, Option<Uuid>, Option<Uuid>);

/// The market sources `AppState::with_market_sources` takes.
pub type MarketSources = Vec<(
    xc_rate::domain::pricing::market_quote::MarketSource,
    Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource>,
)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowRow {
    pub status: String,
    pub reasons: Vec<String>,
    pub anchor_batch_id: Option<Uuid>,
    pub tier_set_id: Option<Uuid>,
}

// ------------------------------------------------------------ the FOLLOW publisher, built from the real services

pub fn drafts(pool: &PgPool) -> Arc<RateDraftService> {
    Arc::new(RateDraftService::new(
        pool.clone(),
        make_approval_service(pool.clone()),
    ))
}

/// `MarketPricingLoop` as `bootstrap/mod.rs` would build it: `RateDraftService` + `PublishService` behind one
/// `TierSetPublisher`, `BranchPush` over the Pg stores, and a `MarketRefresh` with no source (the tests seed the
/// market themselves and only call `plan_once` / `plan_currency`).
pub fn pricing_loop(pool: &PgPool) -> MarketPricingLoop {
    let publisher = Arc::new(TierSetPublisher::new(
        drafts(pool),
        Arc::new(PublishService::new(pool.clone())),
    ));
    let push = Arc::new(BranchPush::new(
        pool.clone(),
        publisher.clone(),
        Arc::new(PgTierSetStore),
        Arc::new(PgBranchPolicyStore),
    ));
    let refresh = Arc::new(MarketRefresh::new(pool.clone(), Vec::new()));
    MarketPricingLoop::new(pool.clone(), refresh, publisher, push)
}

// ------------------------------------------------------------ callers

/// The owner as an `Actor`: EXECUTIVE + ADMIN organisation-wide.
pub fn owner_actor() -> User {
    user_of("owner-1|EXECUTIVE").expect("the owner")
}

/// A caller, sent in the `x-test-as` header as `id|ROLE`.
pub struct Who(pub String);

pub fn owner() -> Who {
    Who("owner-1|EXECUTIVE".into())
}

pub fn rate_manager() -> Who {
    Who(format!("{}|RATE_MANAGER", Uuid::new_v4()))
}

pub fn teller() -> Who {
    Who(format!("{}|TELLER", Uuid::new_v4()))
}

fn user_of(spec: &str) -> Option<User> {
    let (id, role) = spec.split_once('|')?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let mut roles = vec![ScopedRole::new(
        Role::parse(role)?,
        None::<BranchId>,
        from,
        None,
    )];
    if role == "EXECUTIVE" {
        roles.push(ScopedRole::new(Role::Admin, None::<BranchId>, from, None));
    }
    Some(User::new(id, ORG, roles, None).with_branch(Some(BranchId::new("MAIN"))))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

pub struct Answer {
    pub status: StatusCode,
    pub etag: Option<String>,
    pub text: String,
    pub body: Value,
}

impl Answer {
    pub fn ok(&self, what: &str) -> Value {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text);
        self.body.clone()
    }

    pub fn code(&self) -> Value {
        self.body["error"]["code"].clone()
    }

    pub fn refused(&self, status: u16, code: &str, what: &str) {
        assert_eq!(
            self.status.as_u16(),
            status,
            "{what}: status; body {}",
            self.text
        );
        assert_eq!(self.code(), json!(code), "{what}: code; body {}", self.text);
    }
}

pub fn state(room: &Room) -> AppState {
    AppState::new(Arc::new(common::settings(&[])), room.pool.clone())
}

/// One request through the whole app built on `state`. A POST carries `body` plus a fresh `Idempotency-Key`.
/// `if_none_match` is sent as `If-None-Match`.
pub async fn call_in(
    state: AppState,
    who: Option<&Who>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    if_none_match: Option<&str>,
) -> Answer {
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if let Some(who) = who {
        req = req.insert_header((AS, who.0.clone()));
    }
    if let Some(tag) = if_none_match {
        req = req.insert_header((header::IF_NONE_MATCH, tag));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let etag = resp
        .headers()
        .get(header::ETAG)
        .map(|v| v.to_str().expect("an ASCII header").to_owned());
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        etag,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// `POST <route>` naming `branch` in the body.
pub async fn command(room: &Room, who: &Who, route: &str, branch: &str, mut body: Value) -> Answer {
    body["branch"] = json!(branch);
    call_in(
        state(room),
        Some(who),
        Method::POST,
        route,
        Some(body),
        None,
    )
    .await
}

/// `GET /api/v3/views/<name>?branch=<branch>` (plus `&since=<version>` when given).
pub async fn view(room: &Room, who: &Who, name: &str, branch: &str, since: Option<i64>) -> Answer {
    let mut uri = format!("/api/v3/views/{name}?branch={branch}");
    if let Some(v) = since {
        uri.push_str(&format!("&since={v}"));
    }
    call_in(state(room), Some(who), Method::GET, &uri, None, None).await
}

/// `GET /api/public/orgs/XC/branches/<code>/rates`, anonymous, with `If-None-Match` when given.
pub async fn public_rates(room: &Room, branch_code: &str, if_none_match: Option<&str>) -> Answer {
    call_in(
        state(room),
        None,
        Method::GET,
        &format!("/api/public/orgs/{XC}/branches/{branch_code}/rates"),
        None,
        if_none_match,
    )
    .await
}

/// The shared public projection (`published_rates_data`, which the `published_rates` view and the public route
/// both serialise) of `branch`, as JSON.
pub async fn projection(room: &Room, branch: &str) -> Value {
    let data = published_rates_data(&room.pool, &BranchId::new(branch))
        .await
        .unwrap_or_else(|e| panic!("published_rates_data({branch}): {e:?}"));
    serde_json::to_value(&data).expect("serialise")
}

/// Every line of `code` in a projection's `lines`.
pub fn lines_of<'a>(data: &'a Value, code: &str) -> Vec<&'a Value> {
    data["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines is an array: {data}"))
        .iter()
        .filter(|l| l["currency"] == json!(code))
        .collect()
}

// ------------------------------------------------------------ commands the tests share

/// `pricingModeSave {currency, mode: FOLLOW}` at MAIN, as the owner (§15.5).
pub async fn follow_on(room: &Room, code: &str) {
    command(
        room,
        &owner(),
        MODE,
        MAIN,
        json!({ "currency": code, "mode": "FOLLOW" }),
    )
    .await
    .ok("pricingModeSave FOLLOW at MAIN");
}

/// `pricingBranchPolicySave {currency, buyBp, sellBp, follow: true}` at `branch`, as the owner (§15.4).
pub async fn enrol(room: &Room, branch: &str, code: &str, buy_bp: i32, sell_bp: i32) {
    command(
        room,
        &owner(),
        BRANCH_POLICY,
        branch,
        json!({ "currency": code, "buyBp": buy_bp, "sellBp": sell_bp, "follow": true }),
    )
    .await
    .ok("pricingBranchPolicySave follow");
}

/// MAIN with a policy, the BoT USD base, FOLLOW on for USD and a fresh TT batch (PV-1's quote) fetched
/// `age_minutes` before `now`. Answers the batch id.
pub async fn follow_usd_at_main(room: &Room, now: DateTime<Utc>, age_minutes: i64) -> Uuid {
    room.policy(MAIN).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;
    follow_on(room, "USD").await;
    room.tt(now - minutes(age_minutes), &[("USD", PV1_TT.0, PV1_TT.1)])
        .await
}

/// PV-7's adjustment of PV-1's tiers at +5 / −5 bp (`H:` §7). Tier 1 is `H:` §7's own example; tiers 2–4 are
/// derived from H: §7's formula, d = 2 (tier 1 checks: 33.23 × 1.0005 = 33.246615 → 33.24; 33.87 × 0.9995 =
/// 33.853065 → 33.86):
/// buy  33.24 × 1.0005 = 33.25662 → 33.25; 33.26 × 1.0005 = 33.27663 → 33.27; 33.29 × 1.0005 = 33.306645 → 33.30
/// sell 33.86 × 0.9995 = 33.84307 → 33.85; 33.84 × 0.9995 = 33.82308 → 33.83; 33.80 × 0.9995 = 33.7831 → 33.79
/// (monotonic as it stands: no `max`/`min` correction needed).
pub const PV7_TIERS: [(&str, &str); 4] = [
    ("33.24", "33.86"),
    ("33.25", "33.85"),
    ("33.27", "33.83"),
    ("33.30", "33.79"),
];

/// PV-15b's setting: branch `b` (the caller has created it and given it a policy) enrolled for USD at +5 / −5 bp;
/// MAIN publishes PV-1's tiers from a fresh TT (10 minutes old) and B follows; then a new fresh TT batch
/// 33.60 / 34.30 (1 minute old) moves tier 1 by 1.2 % and MAIN's latest plan is `BLOCKED` `LARGE_MOVE` (PV-12). TT
/// stays current, so the hide reason can only be `BLOCKED` (`FollowVisibility::of` checks `STALE` first).
pub async fn b_follows_and_main_blocks(room: &Room, b: &str, now: DateTime<Utc>) {
    follow_usd_at_main(room, now, 10).await;
    enrol(room, b, "USD", 5, -5).await;
    let first = pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));
    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(
        room.published_tiers(b, "USD").await.first().copied(),
        Some((dec("33.24"), dec("33.86"))),
        "B follows (PV-14)"
    );
    room.tt(now - minutes(1), &[("USD", "33.60", "34.30")])
        .await;
    pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()));
    let row = room.follow_result("USD").await.expect("a result row");
    assert_eq!(row.status, "BLOCKED", "MAIN's latest plan: {row:?}");
}

// ------------------------------------------------------------ a fake market source

/// A market source answering fixed rows as `MarketRateSource` emits them (`market_rate_source.rs:107-115`), stamped
/// with the fetch's own time. Counts its calls.
pub struct FakeMarket {
    pub label: &'static str,
    pub rows: Vec<Value>,
    pub calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeMarket {
    pub fn new(label: &'static str, rows: &[(&str, &str, &str)]) -> Self {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        Self {
            label,
            rows: rows
                .iter()
                .map(|(code, buy, sell)| {
                    json!({
                        "code": code, "band": "L", "denoms": "100-50", "buy": buy, "sell": sell,
                        "decimals": 2, "source_updated_at": today,
                    })
                })
                .collect(),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl xc_rate::domain::rate::entities::rate_source::RateSource for FakeMarket {
    fn fetch(
        &self,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = pos_error::AppResult<
                        xc_rate::domain::rate::entities::rate_source::ReferenceRates,
                    >,
                > + Send
                + '_,
        >,
    > {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let out = xc_rate::domain::rate::entities::rate_source::ReferenceRates {
            source: self.label.to_owned(),
            fetched_at: Utc::now().to_rfc3339(),
            kind: "MARKET".to_owned(),
            rows: self.rows.clone(),
        };
        Box::pin(async move { Ok(out) })
    }
}
