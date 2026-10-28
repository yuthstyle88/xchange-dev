//! The harness of the V.9 MANUAL-approve api tests (pricing-tiers.md §15.3, §15.4, §15.6, §15.10):
//! `v9_pricing_approve_test.rs`, `v9_pricing_push_test.rs`, `v9_pricing_quote_test.rs` and `v9_pricing_bands_test.rs`.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! A copy of `pricing_common`'s room and request seam (that module is not ours to edit), plus the V.9 seeding:
//! MAIN (seeded by the migrations) and the branches a test names, each with policy version 1; TT reference batches
//! (`MARKET_TT`, per one unit) with their `market_feed_status` row; BoT batches; and readers for `pricing_tier_sets`,
//! `pricing_branch_policy` and the current publication's tier lines.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Every test fails
//! without it; none skips. Each `Room` is a schema of its own with `api/migrations/` applied.
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
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

#[path = "../common/mod.rs"]
mod common;

pub const MUST_FINISH: Duration = Duration::from_secs(30);
pub const AS: &str = "x-test-as";
pub const HOST: &str = "xc.test";
pub const SAME_ORIGIN: &str = "http://xc.test";
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const MAIN: &str = "MAIN";
pub const APPROVE: &str = "/api/pricing/approve";
pub const BRANCH_POLICY: &str = "/api/pricing/branch-policy";

/// PV-1's tiers (`H:` §4.4: TT 33.20/33.90, d = 2, defaults), per one USD.
pub const PV1_TIERS: [(&str, &str); 4] = [
    ("33.23", "33.87"),
    ("33.24", "33.86"),
    ("33.26", "33.84"),
    ("33.29", "33.80"),
];

/// The TT line PV-1's tiers were suggested from (`H:` §4.4).
pub const USD_TT: (&str, &str, &str) = ("USD", "33.20", "33.90");

pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap_or_else(|e| panic!("invalid decimal '{text}': {e}"))
}

/// A JSON decimal string as a `Decimal` (so `"1.5"` and `"1.5000"` compare equal).
pub fn dec_of(v: &Value) -> Decimal {
    dec(v
        .as_str()
        .unwrap_or_else(|| panic!("a decimal string, got {v}")))
}

pub fn decs(tiers: [(&str, &str); 4]) -> Vec<(Decimal, Decimal)> {
    tiers.iter().map(|(b, s)| (dec(b), dec(s))).collect()
}

// ------------------------------------------------------------ database

pub struct Room {
    pub pool: PgPool,
    schema: String,
}

/// One `pricing_tier_sets` row.
#[derive(Debug, Clone)]
pub struct TierSetRow {
    pub id: Uuid,
    pub request_id: String,
    pub branch: String,
    pub source: String,
    pub base_tier_set_id: Option<Uuid>,
    pub tiers: Value,
    pub anchor: Option<Value>,
    pub draft_id: Option<Uuid>,
}

impl TierSetRow {
    /// `tiers` (jsonb `[{tier,buy,sell}]`) as `(buy, sell)` in tier order.
    pub fn prices(&self) -> Vec<(Decimal, Decimal)> {
        let mut rows: Vec<&Value> = self
            .tiers
            .as_array()
            .unwrap_or_else(|| panic!("tiers is an array: {}", self.tiers))
            .iter()
            .collect();
        rows.sort_by_key(|t| t["tier"].as_i64().unwrap_or(0));
        rows.iter()
            .map(|t| (dec_of(&t["buy"]), dec_of(&t["sell"])))
            .collect()
    }
}

type TierSetTuple = (
    Uuid,
    String,
    String,
    String,
    Option<Uuid>,
    Value,
    Option<Value>,
    Option<Uuid>,
);

type PubLineTuple = (
    Uuid,
    String,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Value,
);

/// One published line of a currency in a branch's current publication.
#[derive(Debug, Clone)]
pub struct PubLine {
    pub id: Uuid,
    pub value_tier: String,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    pub lower_exclusive_thb: Option<Decimal>,
    pub upper_inclusive_thb: Option<Decimal>,
    pub snapshot: Value,
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
        let schema = unique("v9price");
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

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    pub async fn create_branch(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid) ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("create branch");
    }

    /// Policy version 1 of `branch` (`pricing_common::policy_v1`'s shape): adjust −0.5 / +0.4, guards ±5 % for USD
    /// and EUR, and, when `value_tiers_on`, the policy bands 20 000 / 100 000 / 500 000 THB with margins T1 0/0 …
    /// T4 +0.3/−0.3. Those bands are deliberately not V.9's 10 000 / 50 000 / 200 000, so a test can tell which
    /// table a MANUAL publication used (§15.3: `rate_policy_value_tiers` stay for AUTO currencies only).
    pub async fn policy(&self, branch: &str, value_tiers_on: bool) -> Uuid {
        let g = json!({ "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
                        "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } });
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_policy_versions \
               (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, valid_from, valid_to, \
                approved_by, reason, value_tiers_on) \
             VALUES (gen_random_uuid(), $1, 1, -0.5, 0.4, $2, now() - interval '1 day', NULL, 'owner-1', 'seeded', $3) \
             RETURNING id",
        )
        .bind(branch)
        .bind(json!({ "USD": g.clone(), "EUR": g }))
        .bind(value_tiers_on)
        .fetch_one(&self.pool)
        .await
        .expect("insert rate_policy_versions");
        for (no, upper, buy, sell) in [
            (1i16, Some("20000.00"), "0.0000", "0.0000"),
            (2, Some("100000.00"), "0.1000", "-0.1000"),
            (3, Some("500000.00"), "0.2000", "-0.2000"),
            (4, None, "0.3000", "-0.3000"),
        ] {
            sqlx::query(
                "INSERT INTO rate_policy_value_tiers (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
                 VALUES ($1, $2, $3::numeric, $4::numeric, $5::numeric)",
            )
            .bind(id)
            .bind(no)
            .bind(upper)
            .bind(buy)
            .bind(sell)
            .execute(&self.pool)
            .await
            .expect("insert rate_policy_value_tiers");
        }
        id
    }

    /// A branch other than MAIN, with its policy version 1 (value tiers off).
    pub async fn branch(&self, branch: &str) {
        self.create_branch(branch).await;
        self.policy(branch, false).await;
    }

    /// The current policy version's id of `branch` (the approve's stale-page guard, which stays: §15.3).
    pub async fn current_version(&self, branch: &str) -> Uuid {
        sqlx::query_scalar(
            "SELECT id FROM rate_policy_versions WHERE branch_id = $1 AND valid_to IS NULL ORDER BY version DESC LIMIT 1",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("{branch} has a current policy version: {e}"))
    }

    /// One reference batch of `provider` / `reference_type` fetched at `fetched_at`, one line per
    /// (currency, buy, sell), unit 1. Answers the batch id.
    pub async fn batch_at(
        &self,
        provider: &str,
        reference_type: &str,
        lines: &[(&str, &str, &str)],
        fetched_at: DateTime<Utc>,
    ) -> Uuid {
        let batch: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES (gen_random_uuid(), $1, CURRENT_DATE, $2, $3, 'COMPLETE') RETURNING id",
        )
        .bind(provider)
        .bind(fetched_at)
        .bind(unique("chk"))
        .fetch_one(&self.pool)
        .await
        .expect("insert rate_reference_batches");
        for (currency, buy, sell) in lines {
            sqlx::query(
                "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
                 VALUES (gen_random_uuid(), $1, $2, $3, 1, $4::numeric, $5::numeric, '{}'::jsonb)",
            )
            .bind(batch)
            .bind(*currency)
            .bind(reference_type)
            .bind(*buy)
            .bind(*sell)
            .execute(&self.pool)
            .await
            .expect("insert rate_reference_lines");
        }
        batch
    }

    /// A fresh TT batch (`MARKET_TT`, per one unit), fetched now, and TT's `market_feed_status` row saying that
    /// fetch was the last attempt, with no error (§15.5's "current"). Answers `(batch id, fetched_at)`.
    pub async fn tt(&self, lines: &[(&str, &str, &str)]) -> (Uuid, DateTime<Utc>) {
        let at = whole_second_now();
        let batch = self.batch_at("TT", "MARKET_TT", lines, at).await;
        sqlx::query(
            "INSERT INTO market_feed_status (source, last_attempt_at, last_error) VALUES ('TT', $1, NULL) \
             ON CONFLICT (source) DO UPDATE SET last_attempt_at = EXCLUDED.last_attempt_at, last_error = NULL",
        )
        .bind(at)
        .execute(&self.pool)
        .await
        .expect("upsert market_feed_status");
        (batch, at)
    }

    /// A BoT batch fetched now. Answers its id.
    pub async fn bot(&self, lines: &[(&str, &str, &str)]) -> Uuid {
        // Two batches of one test must not share `fetched_at`, or "newest" is a coin toss.
        let at = Utc::now();
        let id = self.batch_at("BOT", "BOT", lines, at).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        id
    }

    /// A `pricing_branch_policy` row written directly (the command is `pricingBranchPolicySave`, pinned in
    /// `v9_pricing_push_test.rs`; tests that only need the row seed it here).
    pub async fn seed_branch_policy(
        &self,
        branch: &str,
        currency: &str,
        buy_bp: Option<i32>,
        sell_bp: Option<i32>,
        own: bool,
    ) {
        sqlx::query(
            "INSERT INTO pricing_branch_policy (branch_id, currency_code, buy_bp, sell_bp, own, follow, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4, $5, false, 'seed', now()) \
             ON CONFLICT (branch_id, currency_code) DO UPDATE SET buy_bp = EXCLUDED.buy_bp, sell_bp = EXCLUDED.sell_bp, \
               own = EXCLUDED.own, updated_at = EXCLUDED.updated_at",
        )
        .bind(branch)
        .bind(currency)
        .bind(buy_bp)
        .bind(sell_bp)
        .bind(own)
        .execute(&self.pool)
        .await
        .expect("seed pricing_branch_policy");
    }

    /// `(buy_bp, sell_bp, own, follow)` of `branch`'s `pricing_branch_policy` row for `currency` (or `'*'`).
    pub async fn branch_policy(
        &self,
        branch: &str,
        currency: &str,
    ) -> Option<(Option<i32>, Option<i32>, bool, bool)> {
        sqlx::query_as(
            "SELECT buy_bp, sell_bp, own, follow FROM pricing_branch_policy WHERE branch_id = $1 AND currency_code = $2",
        )
        .bind(branch)
        .bind(currency)
        .fetch_optional(&self.pool)
        .await
        .expect("read pricing_branch_policy")
    }

    /// Every `pricing_tier_sets` row of (branch, currency), oldest first.
    pub async fn tier_sets(&self, branch: &str, currency: &str) -> Vec<TierSetRow> {
        let rows: Vec<TierSetTuple> = sqlx::query_as(
            "SELECT id, request_id, branch_id, source, base_tier_set_id, tiers, anchor, draft_id \
             FROM pricing_tier_sets WHERE branch_id = $1 AND currency_code = $2 ORDER BY approved_at, id",
        )
        .bind(branch)
        .bind(currency)
        .fetch_all(&self.pool)
        .await
        .expect("read pricing_tier_sets");
        rows.into_iter()
            .map(
                |(id, request_id, branch, source, base_tier_set_id, tiers, anchor, draft_id)| {
                    TierSetRow {
                        id,
                        request_id,
                        branch,
                        source,
                        base_tier_set_id,
                        tiers,
                        anchor,
                        draft_id,
                    }
                },
            )
            .collect()
    }

    /// The one tier set of (branch, currency); panics unless there is exactly one.
    pub async fn only_tier_set(&self, branch: &str, currency: &str) -> TierSetRow {
        let mut sets = self.tier_sets(branch, currency).await;
        assert_eq!(
            sets.len(),
            1,
            "exactly one pricing_tier_sets row for ({branch}, {currency}): {sets:?}"
        );
        sets.remove(0)
    }

    /// `branches.current_publication_id` of `branch`.
    pub async fn current_publication(&self, branch: &str) -> Option<Uuid> {
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("read branches.current_publication_id")
    }

    /// `currency`'s lines in `branch`'s current publication, lowest band first (the untiered `ALL` line, if any,
    /// sorts first too: its lower bound is NULL).
    pub async fn published(&self, branch: &str, currency: &str) -> Vec<PubLine> {
        let rows: Vec<PubLineTuple> = sqlx::query_as(
            "SELECT l.id, l.value_tier, l.buy_rate, l.sell_rate, l.tier_lower_exclusive_thb, \
                    l.tier_upper_inclusive_thb, l.source_snapshot \
             FROM published_rate_lines l JOIN branches b ON b.current_publication_id = l.publication_id \
             WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = 'ALL' \
             ORDER BY l.tier_lower_exclusive_thb NULLS FIRST, l.value_tier",
        )
        .bind(branch)
        .bind(currency)
        .fetch_all(&self.pool)
        .await
        .expect("read published_rate_lines");
        rows.into_iter()
            .map(|(id, value_tier, buy, sell, lo, hi, snapshot)| PubLine {
                id,
                value_tier,
                buy,
                sell,
                lower_exclusive_thb: lo,
                upper_inclusive_thb: hi,
                snapshot,
            })
            .collect()
    }

    /// `(buy, sell)` of each published line of `currency` at `branch`, lowest band first.
    pub async fn published_prices(&self, branch: &str, currency: &str) -> Vec<(Decimal, Decimal)> {
        self.published(branch, currency)
            .await
            .into_iter()
            .map(|l| {
                (
                    l.buy.expect("a published buy"),
                    l.sell.expect("a published sell"),
                )
            })
            .collect()
    }

    /// The number of drafts of `branch`.
    pub async fn drafts_of(&self, branch: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM rate_drafts WHERE branch_id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("count rate_drafts")
    }
}

pub fn whole_second_now() -> DateTime<Utc> {
    let now = Utc::now();
    Utc.timestamp_opt(now.timestamp(), 0).single().expect("now")
}

// ------------------------------------------------------------ callers

/// A caller, sent in the `x-test-as` header: `id|ROLE` (`pricing_common`'s form: an organisation-wide user who is
/// also the staff session), or `staff:<ROLE>:<branch>` (`v3_quotes_route_test`'s form, through `test_callers`).
pub struct Who(pub String);

/// EXECUTIVE + ADMIN organisation-wide: may change the frame (`may_change_the_frame`).
pub fn owner() -> Who {
    Who("owner-1|EXECUTIVE".into())
}

/// `RatePolicySave` + `RatesDraft` and **no** frame right (`pricing_approve_test.rs`'s
/// `a_rate_manager_may_approve_in_bound`).
pub fn rate_manager() -> Who {
    Who(format!("{}|RATE_MANAGER", Uuid::new_v4()))
}

/// A FRONT teller at `branch`: the desk quote's caller in `v3_quotes_route_test.rs`.
pub fn front(branch: &str) -> Who {
    Who(format!("staff:FRONT:{branch}"))
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
    Some(User::new(id, ORG, roles, None))
}

fn callers_of(spec: &str) -> Callers {
    if let Some(staff) = spec.strip_prefix("staff:") {
        let mut parts = staff.splitn(2, ':');
        let role_name = parts.next().expect("a role");
        let branch = parts.next().expect("a branch");
        let roles = test_callers::v2_staff_roles(role_name)
            .unwrap_or_else(|| panic!("unknown role {role_name:?}"));
        let actor = test_callers::staff(
            &format!("employee-{role_name}"),
            roles,
            BranchId::new(branch),
        );
        return test_callers::callers(None, Some(actor));
    }
    let user = user_of(spec);
    Callers {
        user: user.clone(),
        staff_user: user,
    }
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

// ------------------------------------------------------------ requests

pub struct Answer {
    pub status: StatusCode,
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

/// One request through the whole app. A POST carries `body` plus `idempotency_key` (a fresh one when `None`).
pub async fn call(
    room: &Room,
    who: &Who,
    method: Method,
    uri: &str,
    body: Option<Value>,
    idempotency_key: Option<&str>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
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
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.0.clone()));
    if let Some(body) = &body {
        let key = idempotency_key
            .map(str::to_owned)
            .unwrap_or_else(|| unique("key"));
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// `POST <route>` naming `branch` in the body (the seam's key, `command.rs` step 5).
pub async fn command(
    room: &Room,
    who: &Who,
    route: &str,
    branch: &str,
    mut body: Value,
    key: Option<&str>,
) -> Answer {
    body["branch"] = json!(branch);
    call(room, who, Method::POST, route, Some(body), key).await
}

// ------------------------------------------------------------ approve

/// One `pricingApprove` row: `{currency, tiers: [{tierNo, buy, sell}] × 4, tierSetId?}`.
pub fn row(currency: &str, tiers: [(&str, &str); 4], tier_set_id: Option<&str>) -> Value {
    let mut r = json!({
        "currency": currency,
        "tiers": tiers.iter().enumerate()
            .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buy": b, "sell": s }))
            .collect::<Vec<_>>(),
    });
    if let Some(id) = tier_set_id {
        r["tierSetId"] = json!(id);
    }
    r
}

/// `pricingApprove` at `branch` with `rows`, on the branch's current policy version (§15.3: the stale-page guard
/// stays) and the Idempotency-Key `key` (a fresh one when `None`).
pub async fn approve(
    room: &Room,
    who: &Who,
    branch: &str,
    rows: Vec<Value>,
    key: Option<&str>,
) -> Answer {
    let version = room.current_version(branch).await;
    command(
        room,
        who,
        APPROVE,
        branch,
        json!({ "policyVersionId": version.to_string(), "rows": rows, "reason": "V.9 approve test" }),
        key,
    )
    .await
}

/// The answer's `currencies[]` entry for `code` (§15.8a).
pub fn currency_result(answer: &Value, code: &str) -> Value {
    answer["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("§15.8a: the answer has `currencies`: {answer}"))
        .iter()
        .find(|c| c["currency"] == json!(code))
        .cloned()
        .unwrap_or_else(|| panic!("§15.8a: `currencies` has {code}: {answer}"))
}

/// `currencies[code].branches[]` entry for `branch` (§15.8a, MAIN only).
pub fn branch_result(answer: &Value, code: &str, branch: &str) -> Value {
    let c = currency_result(answer, code);
    c["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("§15.8a: MAIN's answer has `branches` for {code}: {c}"))
        .iter()
        .find(|b| b["branchId"] == json!(branch))
        .cloned()
        .unwrap_or_else(|| panic!("§15.4: MAIN's answer lists {branch}: {c}"))
}

/// The `tierSetId` of a PUBLISHED currency result, as a uuid.
pub fn tier_set_id_of(result: &Value) -> Uuid {
    Uuid::from_str(
        result["tierSetId"]
            .as_str()
            .unwrap_or_else(|| panic!("§15.8a: a `tierSetId`: {result}")),
    )
    .unwrap_or_else(|e| panic!("tierSetId is a uuid: {e}; {result}"))
}

/// A room with MAIN on policy version 1 (value tiers on, the policy's own bands), PV-1's TT line, and a BoT USD line
/// (32.45/32.90, `pricing_common`'s worked example). A MANUAL approve does not read BoT (§15.3); the line is there so
/// that today's margin model gets past its "no reference base" refusal and each test fails at what it pins.
pub async fn main_room() -> Room {
    let room = Room::start().await;
    room.policy(MAIN, true).await;
    room.tt(&[USD_TT]).await;
    room.bot(&[("USD", "32.4500", "32.9000")]).await;
    room
}
