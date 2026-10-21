//! The harness of the pricing-tiers api tests (pricing-tiers.md §6, §11): `pricing_configure_test.rs` (PT-5),
//! `pricing_approve_test.rs` (PT-6) and `pricing_board_test.rs` (PT-7).
//! Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not edit, delete or weaken;
//! no ticket's File scope includes this file.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Every test fails
//! without it; none skips. Each `Room` is a schema of its own with `api/migrations/` applied.
//!
//! The branch `XC-B01` is seeded with §12.3's worked example: policy version 1, adjust −0.5 / +0.4, guards ±5 % for
//! USD and EUR, value tiers ON with bands 20 000 / 100 000 / 500 000 THB and margins T1 0/0, T2 +0.1/−0.1,
//! T3 +0.2/−0.2, T4 +0.3/−0.3; BoT USD 32.4500 / 32.9000 and EUR 38.0000 / 38.6000 (unit 1).
#![allow(dead_code)]

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
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

#[path = "../common/mod.rs"]
mod common;

pub const MUST_FINISH: Duration = Duration::from_secs(30);
pub const AS: &str = "x-test-as";
pub const HOST: &str = "xc.test";
pub const SAME_ORIGIN: &str = "http://xc.test";
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
pub const B01: &str = "XC-B01";

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("pricing");
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

    /// The worked-example branch (see the module doc): the branch, policy version 1 and the BoT batch.
    /// Answers version 1's id.
    pub async fn worked_example(&self) -> Uuid {
        self.create_branch(B01).await;
        let v1 = self.policy_v1(B01, true).await;
        self.bot_batch(
            &[("USD", "32.4500", "32.9000"), ("EUR", "38.0000", "38.6000")],
            1,
        )
        .await;
        v1
    }

    pub async fn policy_v1(&self, branch: &str, value_tiers_on: bool) -> Uuid {
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

    /// One batch of `provider` / `reference_type` fetched `hours_ago`, one line per (currency, buy, sell), unit 1.
    pub async fn batch(
        &self,
        provider: &str,
        reference_type: &str,
        lines: &[(&str, &str, &str)],
        hours_ago: i32,
    ) {
        let batch: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES (gen_random_uuid(), $1, CURRENT_DATE, now() - make_interval(hours => $2), $3, 'COMPLETE') RETURNING id",
        )
        .bind(provider)
        .bind(hours_ago)
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
    }

    pub async fn bot_batch(&self, lines: &[(&str, &str, &str)], hours_ago: i32) {
        self.batch("BOT", "BOT", lines, hours_ago).await
    }

    pub async fn tt_batch(&self, lines: &[(&str, &str, &str)], hours_ago: i32) {
        self.batch("TT", "MARKET_TT", lines, hours_ago).await
    }

    pub async fn sr_batch(&self, lines: &[(&str, &str, &str)], hours_ago: i32) {
        self.batch("SR", "MARKET_SR", lines, hours_ago).await
    }

    /// `currency`'s four override margins on `version` (`rate_policy_currency_tiers`).
    pub async fn override_margins(
        &self,
        version: Uuid,
        currency: &str,
        margins: [(&str, &str); 4],
    ) {
        for (i, (buy, sell)) in margins.iter().enumerate() {
            sqlx::query(
                "INSERT INTO rate_policy_currency_tiers (policy_version_id, currency_code, tier_no, buy_margin_pct, sell_margin_pct) \
                 VALUES ($1, $2, $3, $4::numeric, $5::numeric)",
            )
            .bind(version)
            .bind(currency)
            .bind((i + 1) as i16)
            .bind(*buy)
            .bind(*sell)
            .execute(&self.pool)
            .await
            .expect("insert rate_policy_currency_tiers");
        }
    }

    /// `currency`'s settings row on `version` (`rate_policy_currency_settings`).
    pub async fn settings(
        &self,
        version: Uuid,
        currency: &str,
        min_spread: Option<&str>,
        anchor: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO rate_policy_currency_settings (policy_version_id, currency_code, min_spread_pct, anchor_source) \
             VALUES ($1, $2, $3::numeric, $4)",
        )
        .bind(version)
        .bind(currency)
        .bind(min_spread)
        .bind(anchor)
        .execute(&self.pool)
        .await
        .expect("insert rate_policy_currency_settings");
    }

    /// `(id, version, valid_to IS NULL)` of every policy version of `branch`, oldest first.
    pub async fn versions(&self, branch: &str) -> Vec<(Uuid, i32, bool)> {
        sqlx::query_as(
            "SELECT id, version, valid_to IS NULL FROM rate_policy_versions WHERE branch_id = $1 ORDER BY version",
        )
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .expect("versions")
    }

    /// The current version's id of `branch`.
    pub async fn current_version(&self, branch: &str) -> Uuid {
        let current: Vec<Uuid> = self
            .versions(branch)
            .await
            .into_iter()
            .filter(|v| v.2)
            .map(|v| v.0)
            .collect();
        assert_eq!(current.len(), 1, "exactly one current version: {current:?}");
        current[0]
    }

    /// `(currency, tier_no, buy_margin_pct, sell_margin_pct)` of `version`'s override rows.
    pub async fn override_rows(&self, version: Uuid) -> Vec<(String, i16, Decimal, Decimal)> {
        sqlx::query_as(
            "SELECT currency_code::text, tier_no, buy_margin_pct, sell_margin_pct FROM rate_policy_currency_tiers \
             WHERE policy_version_id = $1 ORDER BY currency_code, tier_no",
        )
        .bind(version)
        .fetch_all(&self.pool)
        .await
        .expect("override rows")
    }

    /// `(currency, min_spread_pct, anchor_source)` of `version`'s settings rows.
    pub async fn settings_rows(
        &self,
        version: Uuid,
    ) -> Vec<(String, Option<Decimal>, Option<String>)> {
        sqlx::query_as(
            "SELECT currency_code::text, min_spread_pct, anchor_source FROM rate_policy_currency_settings \
             WHERE policy_version_id = $1 ORDER BY currency_code",
        )
        .bind(version)
        .fetch_all(&self.pool)
        .await
        .expect("settings rows")
    }

    /// `(value_tier, buy_rate, sell_rate)` of `currency` in `branch`'s current publication, by tier.
    pub async fn current_rates(
        &self,
        branch: &str,
        currency: &str,
    ) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT l.value_tier, l.buy_rate::text, l.sell_rate::text FROM published_rate_lines l \
             JOIN branches b ON b.current_publication_id = l.publication_id \
             WHERE b.id = $1 AND l.currency_code = $2 ORDER BY l.value_tier",
        )
        .bind(branch)
        .bind(currency)
        .fetch_all(&self.pool)
        .await
        .expect("current rates")
    }
}

// ------------------------------------------------------------ callers

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
    Some(User::new(id, ORG, roles, None))
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

/// `POST <route>` naming `branch` in the body.
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

/// `GET /api/v3/views/<view>?branch=<branch>`'s `data`.
pub async fn view(room: &Room, who: &Who, name: &str, branch: &str) -> Answer {
    call(
        room,
        who,
        Method::GET,
        &format!("/api/v3/views/{name}?branch={branch}"),
        None,
        None,
    )
    .await
}
