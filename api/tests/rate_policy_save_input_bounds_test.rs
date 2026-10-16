//! Ticket #2041 — `POST /api/rate-policies` (`ratePolicySave`) input bounds and `onStale` validation.
//!
//! Verifies the behaviour table from #2041:
//! - Row 1: `onStale: "FOO"` -> 400 `validationFailed`, field `onStale`
//! - Row 2: `buyAdjustPct: "100000"` -> 400 `validationFailed`, field `buyAdjustPct`
//! - Row 3: `sellAdjustPct: "100000"` -> 400 `validationFailed`, field `sellAdjustPct`
//! - Row 4: `staleLimitThb: "1000000000000000000"` -> 400 `validationFailed`, field `staleLimitThb`
//! - Row 5: `sharpMoveAlertPct: "100000"` -> 400 `validationFailed`, field `sharpMoveAlertPct`
//! - Row 6: buy `"99999.9999"`, sell `"-99999.9999"`, stale `"999999999999999999.99"`,
//!   sharp `"99999.9999"`, onStale `"STOP"` -> 200
//! - Every refusal leaves the `rate_policy_versions` row count unchanged.

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
        let schema = unique("ratepolicysavebounds");
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

// ------------------------------------------------------------ behaviour table rows

#[actix_web::test]
async fn row1_on_stale_unknown_value_answers_400_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    let before = db.version_count(B01).await;

    let mut body = frame();
    body["onStale"] = json!("FOO");
    let a = save(&db, OWNER, B01, body).await;

    assert_validation_failed(&a, "onStale");
    assert_eq!(
        db.version_count(B01).await,
        before,
        "rate_policy_versions row count must be unchanged on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn row2_buy_adjust_pct_out_of_range_answers_400_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    let before = db.version_count(B01).await;

    let mut body = frame();
    body["buyAdjustPct"] = json!("100000");
    let a = save(&db, OWNER, B01, body).await;

    assert_validation_failed(&a, "buyAdjustPct");
    assert_eq!(
        db.version_count(B01).await,
        before,
        "rate_policy_versions row count must be unchanged on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn row3_sell_adjust_pct_out_of_range_answers_400_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    let before = db.version_count(B01).await;

    let mut body = frame();
    body["sellAdjustPct"] = json!("100000");
    let a = save(&db, OWNER, B01, body).await;

    assert_validation_failed(&a, "sellAdjustPct");
    assert_eq!(
        db.version_count(B01).await,
        before,
        "rate_policy_versions row count must be unchanged on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn row4_stale_limit_thb_out_of_range_answers_400_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    let before = db.version_count(B01).await;

    let mut body = frame();
    body["staleLimitThb"] = json!("1000000000000000000");
    let a = save(&db, OWNER, B01, body).await;

    assert_validation_failed(&a, "staleLimitThb");
    assert_eq!(
        db.version_count(B01).await,
        before,
        "rate_policy_versions row count must be unchanged on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn row5_sharp_move_alert_pct_out_of_range_answers_400_validation_failed_and_writes_nothing() {
    let db = TestDb::start().await;
    let before = db.version_count(B01).await;

    let mut body = frame();
    body["sharpMoveAlertPct"] = json!("100000");
    let a = save(&db, OWNER, B01, body).await;

    assert_validation_failed(&a, "sharpMoveAlertPct");
    assert_eq!(
        db.version_count(B01).await,
        before,
        "rate_policy_versions row count must be unchanged on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn row5b_sharp_move_alert_pct_number_out_of_range_answers_400_validation_failed_and_writes_nothing()
 {
    let db = TestDb::start().await;
    let before = db.version_count(B01).await;

    let mut body = frame();
    body["sharpMoveAlertPct"] = json!(100000);
    let a = save(&db, OWNER, B01, body).await;

    assert_validation_failed(&a, "sharpMoveAlertPct");
    assert_eq!(
        db.version_count(B01).await,
        before,
        "rate_policy_versions row count must be unchanged on refusal"
    );
    db.finish().await;
}

#[actix_web::test]
async fn row6_largest_values_that_fit_answer_200() {
    let db = TestDb::start().await;

    let mut body = frame();
    body["buyAdjustPct"] = json!("99999.9999");
    body["sellAdjustPct"] = json!("-99999.9999");
    body["staleLimitThb"] = json!("999999999999999999.99");
    body["sharpMoveAlertPct"] = json!("99999.9999");
    body["onStale"] = json!("STOP");
    let a = save(&db, OWNER, B01, body).await;

    assert_eq!(
        a.status,
        StatusCode::OK,
        "largest values that fit must answer 200: {}",
        a.text
    );
    assert_eq!(a.body["version"], json!(1), "{}", a.text);
    assert_eq!(db.version_count(B01).await, 1);
    db.finish().await;
}
