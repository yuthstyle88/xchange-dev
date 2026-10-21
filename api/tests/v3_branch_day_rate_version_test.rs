//! Tests for CA-8b (#1274): rateVersion RFC 3339 parsing and instant matching on `dayStartService`.
//!
//! (a) read staff_published_rates through the real view, send its publishedAt VERBATIM as rateVersion: success;
//! (b) the same instant written with millis and Z: success;
//! (c) a different instant: the stale refusal;
//! (d) garbage: validationFailed.

mod common;
mod rates_re4c_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MANAGER: &str = "BRANCH_MANAGER@XC-B01";

// Fixed instant with milliseconds
const RATE_INSTANT: &str = "2026-10-12T03:00:00.123Z";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn user_id(spec: &str) -> &'static str {
    match spec {
        MANAGER => "00000000-0000-4000-8000-0000000000b1",
        _ => "00000000-0000-4000-8000-0000000000b7",
    }
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
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("v3ratever");
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

        let scoped = options.clone().options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(scoped)
            .await
            .expect("connect scoped to test schema");

        let migrator = Migrator::new(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations"
        )))
        .await
        .expect("load migrations");
        migrator.run(&pool).await.expect("apply migrations");

        let db = Self { pool, schema };
        db.exec("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{}', '{ORG}', 'Manager', 'ACTIVE', 'test')",
            user_id(MANAGER)
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CU-LIVE-1', '{ORG}', 'CU-LIVE-1', 'Somchai', 'LIVE', now())"
        ))
        .await;
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn phase_is(&self, branch: &str, phase: &str) {
        self.exec(&format!(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('{branch}', '{phase}') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase"
        ))
        .await;
    }

    async fn open_around_now(&self, branch: &str) {
        self.exec(&format!(
            "UPDATE branches b SET opening_hours = (SELECT jsonb_object_agg(d, jsonb_build_array(jsonb_build_object( \
               'open', to_char((now() AT TIME ZONE b.timezone) - interval '1 hour', 'HH24:MI'), \
               'close', to_char((now() AT TIME ZONE b.timezone) + interval '2 hours', 'HH24:MI')))) \
             FROM unnest(ARRAY['mon','tue','wed','thu','fri','sat','sun']) d) WHERE b.id = '{branch}'"
        ))
        .await;
    }

    async fn rates_published(&self, branch: &str, instant: &str) {
        rates_re4c_common::set_counter_rate(&self.pool, branch, "USD", "35.0", "36.0").await;
        self.exec(&format!(
            "UPDATE rate_publications SET published_at = '{instant}' \
             WHERE id = (SELECT current_publication_id FROM branches WHERE id = '{branch}')"
        ))
        .await;
    }

    async fn finish(self) {
        let schema = self.schema;
        let pool = self.pool;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"
        )))
        .execute(&pool)
        .await
        .expect("drop test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "BRANCH_MANAGER" => Role::BranchManager,
        _ => panic!("unexpected role in test: {role}"),
    };
    Some(User::new(
        user_id(spec),
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
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

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
    }
}

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn post(db: &TestDb, who: &str, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("k")))
        .set_payload(body.to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

// ------------------------------------------------------------ tests

/// (a) read staff_published_rates through the real view, send its publishedAt VERBATIM as rateVersion: success.
#[actix_web::test]
async fn test_rate_version_verbatim_from_view_succeeds() {
    let db = TestDb::start().await;
    db.open_around_now(B01).await;
    db.rates_published(B01, RATE_INSTANT).await;
    db.phase_is(B01, "READY").await;

    // Read staff_published_rates through the real view
    let view_res = get(
        &db,
        MANAGER,
        &format!("/api/v3/views/staff_published_rates?branch={B01}"),
    )
    .await;
    assert!(
        view_res.status.is_success(),
        "reading staff_published_rates view must succeed: {}",
        view_res.text
    );

    let published_at_verbatim = view_res.body["data"]["publication"]["publishedAt"]
        .as_str()
        .expect("data.publication.publishedAt string")
        .to_string();

    // Verify it formatted with timezone offset (+00:00) per views/published_rates.rs pat.to_rfc3339()
    assert!(
        published_at_verbatim.contains("+00:00"),
        "publishedAt from view must have +00:00 format: {published_at_verbatim}"
    );

    // Send publishedAt VERBATIM as rateVersion
    let start_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": published_at_verbatim,
        }),
    )
    .await;

    assert!(
        start_res.status.is_success(),
        "(a) verbatim publishedAt from view must succeed: {} {}",
        start_res.status,
        start_res.text
    );
    assert_eq!(start_res.body["phase"], "OPEN");

    db.finish().await;
}

/// (b) the same instant written with millis and Z: success.
#[actix_web::test]
async fn test_rate_version_same_instant_millis_and_z_succeeds() {
    let db = TestDb::start().await;
    db.open_around_now(B01).await;
    db.rates_published(B01, RATE_INSTANT).await;
    db.phase_is(B01, "READY").await;

    // Send the same instant written with millis and Z: RATE_INSTANT = "2026-10-12T03:00:00.123Z"
    let start_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": RATE_INSTANT,
        }),
    )
    .await;

    assert!(
        start_res.status.is_success(),
        "(b) same instant written with millis and Z must succeed: {} {}",
        start_res.status,
        start_res.text
    );
    assert_eq!(start_res.body["phase"], "OPEN");

    db.finish().await;
}

/// (c) a different instant: the stale refusal.
#[actix_web::test]
async fn test_rate_version_different_instant_gives_stale_refusal() {
    let db = TestDb::start().await;
    db.rates_published(B01, RATE_INSTANT).await;
    db.phase_is(B01, "READY").await;

    // Send a different instant (e.g. 1 hour later)
    let different_instant = "2026-10-12T04:00:00.123Z";
    let start_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": different_instant,
        }),
    )
    .await;

    assert_eq!(
        start_res.status,
        StatusCode::BAD_REQUEST,
        "(c) different instant must return 400 Bad Request"
    );
    assert_eq!(
        start_res.code(),
        "validationFailed",
        "(c) different instant must return validationFailed code"
    );
    assert!(
        start_res.field().is_none(),
        "(c) different instant must not name a field (stale refusal has no field)"
    );
    assert!(
        start_res
            .text
            .contains("ยืนยันอุปกรณ์และเรทที่แสดงให้ตรงกับชุดเรทล่าสุด"),
        "(c) must contain ServiceChecksIncomplete sentence: {}",
        start_res.text
    );

    db.finish().await;
}

/// (d) garbage: validationFailed naming rateVersion.
#[actix_web::test]
async fn test_rate_version_garbage_gives_validation_failed_naming_rate_version() {
    let db = TestDb::start().await;
    db.rates_published(B01, RATE_INSTANT).await;
    db.phase_is(B01, "READY").await;

    // Send garbage that is not an RFC 3339 timestamp
    let start_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": "not-an-rfc3339-timestamp",
        }),
    )
    .await;

    assert_eq!(
        start_res.status,
        StatusCode::BAD_REQUEST,
        "(d) garbage must return 400 Bad Request"
    );
    assert_eq!(
        start_res.code(),
        "validationFailed",
        "(d) garbage must return validationFailed code"
    );
    assert_eq!(
        start_res.field(),
        Some("rateVersion"),
        "(d) garbage must name field rateVersion in error response"
    );

    db.finish().await;
}

/// All four cases (a)–(d) executed sequentially against the same published rate.
#[actix_web::test]
async fn test_rate_version_all_four_ruling_cases_in_sequence() {
    let db = TestDb::start().await;
    db.open_around_now(B01).await;
    db.rates_published(B01, RATE_INSTANT).await;

    // (a) Read through real view, send verbatim (+00:00): success
    db.phase_is(B01, "READY").await;
    let view_res = get(
        &db,
        MANAGER,
        &format!("/api/v3/views/staff_published_rates?branch={B01}"),
    )
    .await;
    let verbatim = view_res.body["data"]["publication"]["publishedAt"]
        .as_str()
        .unwrap();
    let a_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": verbatim,
        }),
    )
    .await;
    assert!(
        a_res.status.is_success(),
        "(a) verbatim must succeed: {}",
        a_res.text
    );
    assert_eq!(a_res.body["phase"], "OPEN");

    // (b) Same instant written with millis and Z: success
    db.phase_is(B01, "READY").await;
    let b_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": RATE_INSTANT,
        }),
    )
    .await;
    assert!(
        b_res.status.is_success(),
        "(b) millis and Z must succeed: {}",
        b_res.text
    );
    assert_eq!(b_res.body["phase"], "OPEN");

    // (c) Different instant: stale refusal (no field)
    db.phase_is(B01, "READY").await;
    let c_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": "2026-10-12T04:00:00.123Z",
        }),
    )
    .await;
    assert_eq!(c_res.status, StatusCode::BAD_REQUEST);
    assert_eq!(c_res.code(), "validationFailed");
    assert!(c_res.field().is_none());
    assert!(c_res.text.contains("ยืนยันอุปกรณ์และเรทที่แสดงให้ตรงกับชุดเรทล่าสุด"));

    // (d) Garbage: validationFailed naming rateVersion
    db.phase_is(B01, "READY").await;
    let d_res = post(
        &db,
        MANAGER,
        "/api/branch-day/start",
        json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": true,
            "rateVersion": "garbage",
        }),
    )
    .await;
    assert_eq!(d_res.status, StatusCode::BAD_REQUEST);
    assert_eq!(d_res.code(), "validationFailed");
    assert_eq!(d_res.field(), Some("rateVersion"));

    db.finish().await;
}
