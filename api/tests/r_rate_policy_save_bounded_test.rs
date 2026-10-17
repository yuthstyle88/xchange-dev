//! R-rate-policy up-front tests (#2054, design `docs/design/blueprint/bounded-inputs.md` §4 rows R-* and R-re) —
//! written before the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller
//! claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `POST /api/rate-policies` (`ratePolicySave`, `rate_policy_save.rs`): every numeric input is bounded by its column,
//! on both sides, and a refusal is 400 `validationFailed` naming the field, with nothing written.
//! - `valueTiers[].buyMarginPct` / `sellMarginPct` → `rate_policy_value_tiers.*_margin_pct numeric(9,4)`
//!   (`20261205151000_rate_tiers_and_starting_prices.sql:8-9`): |v| < 10^5. Main answers 500 at the INSERT.
//! - `buyAdjustPct` / `sellAdjustPct` → `numeric(9,4)` (`20261010120000_rate_engine.sql:30-31`), `staleLimitThb` →
//!   `numeric(20,2)` (`:35`), `sharpMoveAlertPct` → `numeric(9,4) CHECK (> 0)` (`rate_tiers…sql:34-35`): main
//!   (#2041) bounds the magnitude, but a fifth decimal (third for `staleLimitThb`) is rounded silently by the column.
//!
//! The #2041 answers (`rate_policy_save_input_bounds_test.rs`) stay as they are.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const OWNER: &str = "owner";
const OWNER_ID: &str = "owner-1";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The policy save route tests need a real PostgreSQL and do not skip. Point it at \
                 an existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("r_rate_policy");
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
            .expect("apply api/migrations to the test schema");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        Self { pool, schema }
    }

    async fn version_count(&self, branch: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM rate_policy_versions WHERE branch_id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("count policy versions")
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec == OWNER {
        return Sessions {
            owner: Some(Actor::owner(OWNER_ID)),
            staff: None,
        };
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(
            format!("employee-{}", role.as_str()),
            role,
            BranchId::new(branch),
        ),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
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
        .insert_header((AS, who));
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
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn save(db: &TestDb, who: &str, branch: &str, body: Value) -> Answer {
    let mut body = body;
    body["branch"] = json!(branch);
    call(db, who, Method::POST, "/api/rate-policies", Some(body)).await
}

fn frame_guards() -> Value {
    json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        },
        "JPY": {
            "buy": { "min_pct": "-2.0000", "max_pct": "0.5000" },
            "sell": { "min_pct": "-0.5000", "max_pct": "2.0000" }
        }
    })
}

fn frame() -> Value {
    json!({
        "buyAdjustPct": "0",
        "sellAdjustPct": "0",
        "guards": frame_guards(),
        "maxReferenceAgeHours": 24,
        "onStale": "LIMIT",
        "staleLimitThb": "5000.00",
        "reason": "the frame for the test",
    })
}

fn assert_validation_failed(a: &Answer, expected_field: &str) {
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "expected 400 validationFailed on {expected_field}, got {}: {}",
        a.status,
        a.text
    );
    assert_eq!(
        a.body["error"]["code"],
        json!("validationFailed"),
        "expected code=validationFailed on {expected_field}: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"],
        json!(expected_field),
        "expected field={expected_field}: {}",
        a.text
    );
}

// ------------------------------------------------------------ helpers

async fn stored(db: &TestDb, branch: &str, col: &str) -> Option<String> {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT {col}::text FROM rate_policy_versions WHERE branch_id = $1 AND valid_to IS NULL"
    )))
    .bind(branch)
    .fetch_optional(&db.pool)
    .await
    .expect("read the current policy")
    .flatten()
}

async fn tier_rows(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM rate_policy_value_tiers")
        .fetch_one(&db.pool)
        .await
        .expect("count tiers")
}

fn tiers_with(buy: &str, sell: &str) -> Value {
    json!([
        { "tierNo": 1, "upperInclusiveThb": "20000.00", "buyMarginPct": "0.0000", "sellMarginPct": "0.0000" },
        { "tierNo": 2, "upperInclusiveThb": "100000.00", "buyMarginPct": "0.1000", "sellMarginPct": "-0.1000" },
        { "tierNo": 3, "upperInclusiveThb": "500000.00", "buyMarginPct": "0.2000", "sellMarginPct": "-0.2000" },
        { "tierNo": 4, "upperInclusiveThb": null, "buyMarginPct": buy, "sellMarginPct": sell }
    ])
}

fn with_tiers(buy: &str, sell: &str) -> Value {
    let mut body = frame();
    body["valueTiersOn"] = json!(true);
    body["valueTiers"] = tiers_with(buy, sell);
    body
}

async fn refused_nothing_written(db: &TestDb, body: Value, field: &str) {
    let before = db.version_count(B01).await;
    let tiers_before = tier_rows(db).await;
    let a = save(db, OWNER, B01, body).await;
    assert_validation_failed(&a, field);
    assert_eq!(
        db.version_count(B01).await,
        before,
        "no policy version on refusal"
    );
    assert_eq!(tier_rows(db).await, tiers_before, "no tier row on refusal");
}

// ------------------------------------------------------------ value tiers: numeric(9,4)

#[actix_web::test]
async fn a_tier_buy_margin_of_ten_to_the_five_is_400_on_value_tiers() {
    let db = TestDb::start().await;
    refused_nothing_written(&db, with_tiers("100000", "-0.3000"), "valueTiers").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_tier_sell_margin_of_minus_ten_to_the_five_is_400_on_value_tiers() {
    let db = TestDb::start().await;
    refused_nothing_written(&db, with_tiers("0.3000", "-100000"), "valueTiers").await;
    db.finish().await;
}

#[actix_web::test]
async fn the_largest_tier_margins_that_fit_are_not_a_bound_refusal() {
    // The largest margins the column holds pass the bound; whatever answers after that (guards, monotonic bands) is
    // not a 500 and not a bound refusal on `valueTiers` for the magnitude.
    let db = TestDb::start().await;
    let a = save(&db, OWNER, B01, with_tiers("99999.9999", "-99999.9999")).await;
    assert_ne!(
        a.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "never 500: {}",
        a.text
    );
    assert!(
        a.status == StatusCode::OK || a.status == StatusCode::BAD_REQUEST,
        "200 or a business refusal: {} {}",
        a.status,
        a.text
    );
    db.finish().await;
}

// ------------------------------------------------------------ scale: no silent rounding

#[actix_web::test]
async fn a_fifth_decimal_on_buy_adjust_pct_is_refused_not_rounded() {
    let db = TestDb::start().await;
    let mut body = frame();
    body["buyAdjustPct"] = json!("0.12345");
    refused_nothing_written(&db, body, "buyAdjustPct").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_fifth_decimal_on_sell_adjust_pct_is_refused_not_rounded() {
    let db = TestDb::start().await;
    let mut body = frame();
    body["sellAdjustPct"] = json!("-0.12345");
    refused_nothing_written(&db, body, "sellAdjustPct").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_third_decimal_on_stale_limit_thb_is_refused_not_rounded() {
    let db = TestDb::start().await;
    let mut body = frame();
    body["staleLimitThb"] = json!("5000.001");
    refused_nothing_written(&db, body, "staleLimitThb").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_fifth_decimal_on_sharp_move_alert_pct_is_refused_string_or_number() {
    let db = TestDb::start().await;
    let mut body = frame();
    body["sharpMoveAlertPct"] = json!("1.00001");
    refused_nothing_written(&db, body, "sharpMoveAlertPct").await;
    let mut body = frame();
    body["sharpMoveAlertPct"] = json!(1.00001);
    refused_nothing_written(&db, body, "sharpMoveAlertPct").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_sharp_move_alert_pct_that_rounds_to_zero_is_refused() {
    // 0.00001 would be stored as 0.0000 and break CHECK (> 0).
    let db = TestDb::start().await;
    let mut body = frame();
    body["sharpMoveAlertPct"] = json!("0.00001");
    refused_nothing_written(&db, body, "sharpMoveAlertPct").await;
    db.finish().await;
}

// ------------------------------------------------------------ kept: the smallest steps are stored exactly

#[actix_web::test]
async fn the_finest_values_each_column_holds_are_stored_exactly() {
    let db = TestDb::start().await;
    let mut body = frame();
    body["buyAdjustPct"] = json!("-0.0001");
    body["sellAdjustPct"] = json!("0.0001");
    body["staleLimitThb"] = json!("0.01");
    body["sharpMoveAlertPct"] = json!("0.0001");
    let a = save(&db, OWNER, B01, body).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        stored(&db, B01, "buy_adjust_pct").await.as_deref(),
        Some("-0.0001")
    );
    assert_eq!(
        stored(&db, B01, "sell_adjust_pct").await.as_deref(),
        Some("0.0001")
    );
    assert_eq!(
        stored(&db, B01, "stale_limit_thb").await.as_deref(),
        Some("0.01")
    );
    assert_eq!(
        stored(&db, B01, "sharp_move_alert_pct").await.as_deref(),
        Some("0.0001")
    );
    db.finish().await;
}

#[actix_web::test]
async fn extra_zero_decimals_are_accepted_as_the_same_value() {
    let db = TestDb::start().await;
    let mut body = frame();
    body["buyAdjustPct"] = json!("0.250000");
    body["staleLimitThb"] = json!("5000.0000");
    let a = save(&db, OWNER, B01, body).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        stored(&db, B01, "buy_adjust_pct").await.as_deref(),
        Some("0.2500")
    );
    assert_eq!(
        stored(&db, B01, "stale_limit_thb").await.as_deref(),
        Some("5000.00")
    );
    db.finish().await;
}

// ------------------------------------------------------------ syntax: one parse for every field

#[actix_web::test]
async fn text_that_is_not_a_plain_decimal_is_refused_on_its_field() {
    let db = TestDb::start().await;
    for (field, key, text) in [
        ("buyAdjustPct", "buyAdjustPct", "1e2"),
        ("sellAdjustPct", "sellAdjustPct", "+1"),
        ("staleLimitThb", "staleLimitThb", "1,000"),
        ("sharpMoveAlertPct", "sharpMoveAlertPct", ".5"),
    ] {
        let mut body = frame();
        body[key] = json!(text);
        refused_nothing_written(&db, body, field).await;
    }
    db.finish().await;
}
