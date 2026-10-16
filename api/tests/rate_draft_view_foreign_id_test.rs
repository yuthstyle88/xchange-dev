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
use uuid::Uuid;
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
const B02: &str = "XC-B02";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 Point it at an existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3rateforeign");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert XC-B01");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B02', 'B02')")
            .execute(&pool)
            .await
            .expect("insert XC-B02");
        Self { pool, schema }
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

struct HeaderSessions;

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Actor::staff(
        format!("employee-{}", role.as_str()),
        role,
        BranchId::new(branch),
    )
}

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    Sessions {
        owner: None,
        staff: staff_of(spec),
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
        .insert_header((header::HOST, HOST));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
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
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn command(db: &TestDb, who: &str, method: Method, uri: &str, body: Value) -> Answer {
    call(db, who, method, uri, Some(body)).await
}

async fn view(db: &TestDb, who: &str, name_and_query: &str) -> Answer {
    call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/{name_and_query}"),
        None,
    )
    .await
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: body {}", a.text);
    a.body.clone()
}

fn manual_line(currency: &str, buy: &str, sell: &str) -> Value {
    json!({
        "currency": currency,
        "buyMode": "MANUAL",
        "sellMode": "MANUAL",
        "manualBuy": buy,
        "manualSell": sell,
        "reason": "set by hand for the test",
    })
}

async fn create_draft(db: &TestDb, who: &str, branch: Option<&str>, lines: Vec<Value>) -> String {
    let mut body = json!({ "lines": lines });
    if let Some(b) = branch {
        body["branch"] = json!(b);
    }
    let a = command(db, who, Method::POST, "/api/rate-drafts", body).await;
    let body = ok(&a, "create a draft");
    assert_eq!(body["status"], json!("DRAFT"), "a new draft: {}", a.text);
    assert_eq!(body["version"], json!(1), "a new draft: {}", a.text);
    body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the draft's id: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ behaviour table (§8)

#[actix_web::test]
async fn r1_foreign_branch_draft_returns_null_draft_and_empty_lines() {
    let db = TestDb::start().await;
    let b2_draft_id = create_draft(
        &db,
        "owner",
        Some(B02),
        vec![manual_line("USD", "32.10000000", "32.45000000")],
    )
    .await;

    let caller = format!("staff:RATES:{B01}");
    let a = view(
        &db,
        &caller,
        &format!("rate_draft?branch={B01}&id={b2_draft_id}"),
    )
    .await;
    let body = ok(&a, "view rate_draft with foreign branch draft id");

    assert_eq!(
        body["data"]["draft"],
        Value::Null,
        "draft must be null for foreign branch draft: {}",
        a.text
    );
    assert_eq!(
        body["data"]["lines"],
        json!([]),
        "lines must be empty for foreign branch draft: {}",
        a.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn r2_own_branch_draft_returns_draft_and_lines() {
    let db = TestDb::start().await;
    let b1_draft_id = create_draft(
        &db,
        &format!("staff:RATES:{B01}"),
        Some(B01),
        vec![manual_line("USD", "32.10000000", "32.45000000")],
    )
    .await;

    let caller = format!("staff:RATES:{B01}");
    let a = view(
        &db,
        &caller,
        &format!("rate_draft?branch={B01}&id={b1_draft_id}"),
    )
    .await;
    let body = ok(&a, "view rate_draft with own branch draft id");

    assert_eq!(
        body["data"]["draft"]["id"],
        json!(b1_draft_id),
        "draft id must match: {}",
        a.text
    );
    assert_eq!(
        body["data"]["draft"]["branchId"],
        json!(B01),
        "draft branchId must match B01: {}",
        a.text
    );
    let lines = body["data"]["lines"].as_array().expect("lines array");
    assert_eq!(lines.len(), 1, "lines must have 1 element: {}", a.text);
    assert_eq!(lines[0]["currency"], json!("USD"));
    assert_eq!(lines[0]["manualBuy"], json!("32.10000000"));
    assert_eq!(lines[0]["manualSell"], json!("32.45000000"));

    db.finish().await;
}

#[actix_web::test]
async fn r3_no_id_returns_null_draft_and_empty_lines() {
    let db = TestDb::start().await;
    let caller = format!("staff:RATES:{B01}");
    let a = view(&db, &caller, &format!("rate_draft?branch={B01}")).await;
    let body = ok(&a, "view rate_draft with no id");

    assert_eq!(
        body["data"]["draft"],
        Value::Null,
        "draft must be null when no id is given: {}",
        a.text
    );
    assert_eq!(
        body["data"]["lines"],
        json!([]),
        "lines must be empty when no id is given: {}",
        a.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn r4_not_a_uuid_returns_null_draft_and_empty_lines() {
    let db = TestDb::start().await;
    let caller = format!("staff:RATES:{B01}");
    let a = view(
        &db,
        &caller,
        &format!("rate_draft?branch={B01}&id=not-a-uuid"),
    )
    .await;
    let body = ok(&a, "view rate_draft with non-uuid id");

    assert_eq!(
        body["data"]["draft"],
        Value::Null,
        "draft must be null for non-uuid id: {}",
        a.text
    );
    assert_eq!(
        body["data"]["lines"],
        json!([]),
        "lines must be empty for non-uuid id: {}",
        a.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn r5_unknown_uuid_returns_null_draft_and_empty_lines() {
    let db = TestDb::start().await;
    let unknown_id = Uuid::new_v4().to_string();
    let caller = format!("staff:RATES:{B01}");
    let a = view(
        &db,
        &caller,
        &format!("rate_draft?branch={B01}&id={unknown_id}"),
    )
    .await;
    let body = ok(&a, "view rate_draft with unknown uuid");

    assert_eq!(
        body["data"]["draft"],
        Value::Null,
        "draft must be null for unknown uuid: {}",
        a.text
    );
    assert_eq!(
        body["data"]["lines"],
        json!([]),
        "lines must be empty for unknown uuid: {}",
        a.text
    );

    db.finish().await;
}
