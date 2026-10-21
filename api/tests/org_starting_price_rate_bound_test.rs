//! Ticket #2707 — orgStartingPriceSave accepts prices above 100,000,000,
//! and evaluating a draft on them panics (500).
//!
//! Behaviour table from issue #2707 §8:
//! - SP1: USD buy 150000000, sell 150000001, a branch with an active policy and no publication:
//!   400 `validationFailed` on `buy`; no batch, line or price row (main: 500/panic).
//! - SP2: USD buy 100000000, sell 100000000:
//!   200 (kept: the bound is inclusive).
//! - SP3: buy 0 or negative:
//!   400 on `buy` (kept).
//! - SP4: buy > sell:
//!   400 on `prices` (kept).
//! - SP5: a normal pair, e.g. 34.10 / 34.50:
//!   200, and the auto-published draft as today (kept).

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";

const CMD_SAVE: &str = "/api/v3/commands/org_starting_price_save";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
    org: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("sp1ratebound");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to test schema");

        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02'), ('XC-B09', 'B09')",
        )
        .execute(&pool)
        .await
        .expect("insert branches");

        let org: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'XC-B01'")
                .fetch_one(&pool)
                .await
                .expect("the branches' organisation");

        Self { pool, schema, org }
    }

    async fn insert_policy_version(&self, branch: &str, version: i32) -> Uuid {
        let policy_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO rate_policy_versions ( \
                id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
                approved_by, reason, source_version \
             ) VALUES ( \
                $1, $2, $3, 0.0000, 0.0000, \
                '{\"USD\": {\"buy\": {\"min_pct\": \"-1.0000\", \"max_pct\": \"1.0000\"}, \"sell\": {\"min_pct\": \"-1.0000\", \"max_pct\": \"1.0000\"}}}'::jsonb, \
                24, 'LIMIT', 5000.00, now() - interval '1 hour', NULL, \
                'owner-1', 'initial policy for test', NULL \
             )",
        )
        .bind(policy_id)
        .bind(branch)
        .bind(version)
        .execute(&self.pool)
        .await
        .expect("insert rate_policy_version");

        policy_id
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers {
    org: String,
}

fn user_id(spec: &str) -> String {
    let n = match spec {
        "owner" | "EXECUTIVE@*" => 1,
        "admin" | "ADMIN@*" => 2,
        "rates" | "RATES@XC-B01" => 3,
        "teller" | "TELLER@XC-B01" => 4,
        "branch_manager" | "BRANCH_MANAGER@XC-B01" => 5,
        other => panic!("no user id written for {other}"),
    };
    format!("00000000-0000-4000-8000-{n:012}")
}

fn user_of(spec: &str, org: &str) -> Option<AccessActor> {
    let (role, branch) = match spec {
        "owner" => ("EXECUTIVE", "*"),
        "admin" => ("ADMIN", "*"),
        "rates" => ("RATES", "XC-B01"),
        "teller" => ("TELLER", "XC-B01"),
        "branch_manager" => ("BRANCH_MANAGER", "XC-B01"),
        other => other.split_once('@')?,
    };
    let role = match role {
        "EXECUTIVE" => AccessRole::Executive,
        "ADMIN" => AccessRole::Admin,
        "RATES" | "RATE_MANAGER" => AccessRole::RateManager,
        "TELLER" => AccessRole::Teller,
        "BRANCH_MANAGER" => AccessRole::BranchManager,
        "FINANCE" => AccessRole::Finance,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(AccessActor::new(
        &user_id(spec),
        org,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        let org = self.org.clone();
        Box::pin(async move {
            let user = user_of(&spec, &org);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
    }

    fn field(&self) -> &str {
        self.body["error"]["field"]
            .as_str()
            .or_else(|| self.body["field"].as_str())
            .unwrap_or("")
    }
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers {
        org: db.org.clone(),
    });
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

fn entry(currency: &str, buy: Option<&str>, sell: Option<&str>) -> Value {
    json!({
        "currency": currency,
        "buy": buy,
        "sell": sell,
    })
}

fn save_payload(prices: Vec<Value>, reason: &str) -> Value {
    json!({
        "prices": prices,
        "reason": reason,
    })
}

async fn assert_no_starting_price_artifacts(db: &TestDb, context: &str) {
    let prices_count: i64 = sqlx::query_scalar("SELECT count(*) FROM org_starting_prices")
        .fetch_one(&db.pool)
        .await
        .expect("count org_starting_prices");
    assert_eq!(
        prices_count, 0,
        "nothing written to org_starting_prices ({context})"
    );

    let batch_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM rate_reference_batches WHERE provider LIKE 'START:%'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count START batches");
    assert_eq!(
        batch_count, 0,
        "no START reference batch must be written ({context})"
    );

    let line_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM rate_reference_lines WHERE reference_type = 'START'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count START lines");
    assert_eq!(
        line_count, 0,
        "no START reference line must be written ({context})"
    );
}

// ------------------------------------------------------------ tests

/// Behaviour table row SP1: USD buy 150000000, sell 150000001, a branch with an active policy
/// and no publication -> 400 `validationFailed` on `buy`; no batch, line or price row (main: 500/panic).
#[actix_web::test]
async fn sp1_rate_above_100_million_refused_validation_failed_on_buy() {
    let db = TestDb::start().await;

    // Branch XC-B01 has an active policy version, but no publication
    db.insert_policy_version(B01, 1).await;

    let payload = save_payload(
        vec![entry("USD", Some("150000000"), Some("150000001"))],
        "SP1: buy above 100,000,000",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;

    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "SP1: must answer 400 Bad Request, got {}: {}",
        a.status,
        a.text
    );
    assert_eq!(
        a.code(),
        "validationFailed",
        "SP1: code must be validationFailed: {}",
        a.text
    );
    assert_eq!(a.field(), "buy", "SP1: field must be buy: {}", a.text);
    assert_no_starting_price_artifacts(&db, "SP1 buy above bound").await;

    // Also verify sell above bound with valid buy is refused on sell
    let payload_sell = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("150000000"))],
        "SP1: sell above 100,000,000",
    );
    let a_sell = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload_sell)).await;

    assert_eq!(
        a_sell.status,
        StatusCode::BAD_REQUEST,
        "SP1 sell: must answer 400 Bad Request, got {}: {}",
        a_sell.status,
        a_sell.text
    );
    assert_eq!(
        a_sell.code(),
        "validationFailed",
        "SP1 sell: code must be validationFailed: {}",
        a_sell.text
    );
    assert_eq!(
        a_sell.field(),
        "sell",
        "SP1 sell: field must be sell: {}",
        a_sell.text
    );
    assert_no_starting_price_artifacts(&db, "SP1 sell above bound").await;

    db.finish().await;
}

/// Behaviour table row SP2: USD buy 100000000, sell 100000000 -> 200 (kept: the bound is inclusive).
#[actix_web::test]
async fn sp2_bound_is_inclusive_100_million_succeeds() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![entry("USD", Some("100000000"), Some("100000000"))],
        "SP2: bound inclusive at 100,000,000",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;

    assert_eq!(
        a.status,
        StatusCode::OK,
        "SP2: must succeed at bound 100,000,000: {}",
        a.text
    );

    db.finish().await;
}

/// Behaviour table row SP3: buy 0 or negative -> 400 on `buy` (kept).
#[actix_web::test]
async fn sp3_buy_zero_or_negative_refused_on_buy() {
    let db = TestDb::start().await;

    // Case 1: buy 0
    let p_zero = save_payload(
        vec![entry("USD", Some("0"), Some("34.50"))],
        "SP3: buy is zero",
    );
    let a_zero = call(&db, "owner", Method::POST, CMD_SAVE, Some(p_zero)).await;
    assert_eq!(
        a_zero.status,
        StatusCode::BAD_REQUEST,
        "SP3 zero: must answer 400: {}",
        a_zero.text
    );
    assert_eq!(
        a_zero.code(),
        "validationFailed",
        "SP3 zero: code must be validationFailed: {}",
        a_zero.text
    );
    assert_eq!(
        a_zero.field(),
        "buy",
        "SP3 zero: field must be buy: {}",
        a_zero.text
    );
    assert_no_starting_price_artifacts(&db, "SP3 zero buy").await;

    // Case 2: buy negative
    let p_neg = save_payload(
        vec![entry("USD", Some("-1"), Some("34.50"))],
        "SP3: buy is negative",
    );
    let a_neg = call(&db, "owner", Method::POST, CMD_SAVE, Some(p_neg)).await;
    assert_eq!(
        a_neg.status,
        StatusCode::BAD_REQUEST,
        "SP3 negative: must answer 400: {}",
        a_neg.text
    );
    assert_eq!(
        a_neg.code(),
        "validationFailed",
        "SP3 negative: code must be validationFailed: {}",
        a_neg.text
    );
    assert_eq!(
        a_neg.field(),
        "buy",
        "SP3 negative: field must be buy: {}",
        a_neg.text
    );
    assert_no_starting_price_artifacts(&db, "SP3 negative buy").await;

    db.finish().await;
}

/// Behaviour table row SP4: buy > sell -> 400 on `prices` (kept).
#[actix_web::test]
async fn sp4_buy_exceeds_sell_refused_on_prices() {
    let db = TestDb::start().await;

    let payload = save_payload(
        vec![entry("USD", Some("35.00"), Some("30.00"))],
        "SP4: buy > sell",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;

    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "SP4: must answer 400: {}",
        a.text
    );
    assert_eq!(
        a.code(),
        "validationFailed",
        "SP4: code must be validationFailed: {}",
        a.text
    );
    assert_eq!(a.field(), "prices", "SP4: field must be prices: {}", a.text);
    assert_no_starting_price_artifacts(&db, "SP4 buy > sell").await;

    db.finish().await;
}

/// Behaviour table row SP5: a normal pair, e.g. 34.10 / 34.50 -> 200, and the auto-published draft as today (kept).
#[actix_web::test]
async fn sp5_normal_pair_succeeds_and_draft_published() {
    let db = TestDb::start().await;

    // Branch XC-B01 has active policy version, but current_publication_id IS NULL
    db.insert_policy_version(B01, 1).await;
    let initial_pub: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("check initial branch publication");
    assert!(
        initial_pub.is_none(),
        "XC-B01 must be unpublished initially"
    );

    let payload = save_payload(
        vec![entry("USD", Some("34.10"), Some("34.50"))],
        "SP5: normal pair",
    );
    let a = call(&db, "owner", Method::POST, CMD_SAVE, Some(payload)).await;

    assert_eq!(
        a.status,
        StatusCode::OK,
        "SP5: save must succeed: {}",
        a.text
    );

    // XC-B01 now has current_publication_id set
    let new_pub: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("check branch publication after save");
    assert!(
        new_pub.is_some(),
        "XC-B01 must have a current_publication_id after starting price save"
    );

    db.finish().await;
}
