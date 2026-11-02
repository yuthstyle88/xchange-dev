//! LG-3b (#5968, server half of #5951): `CashLimitsData` answers the setter's name (`updatedByName`).
//! Behaviour table from issue #5968 §9, one test per row.

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
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

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
        let schema = unique("clname");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches");
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

async fn insert_user(db: &TestDb, id: &str, name: &str) {
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2::uuid, $3, 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(ORG)
    .bind(name)
    .execute(&db.pool)
    .await
    .expect("insert user");
}

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (left, branch) = spec.split_once('@')?;
    let (role_str, uid) = match left.split_once(':') {
        Some((r, id)) => (r, id.to_owned()),
        None => (left, user_id(spec)),
    };
    let role = match role_str {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let session_branch = Some(branch.clone().unwrap_or_else(|| BranchId::new("MAIN")));
    Some(
        User::new(
            &uid,
            "XC",
            vec![ScopedRole::new(
                role,
                branch,
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
            None,
        )
        .with_branch(session_branch),
    )
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
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

async fn save(db: &TestDb, who: &str, branch: &str, limit: Value) -> Answer {
    call(
        db,
        who,
        Method::PUT,
        "/api/cash-limits",
        Some(json!({ "branch": branch, "countDifferenceLimitThb": limit })),
    )
    .await
}

async fn read(db: &TestDb, who: &str, branch: &str) -> Answer {
    call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/cash_limits?branch={branch}"),
        None,
    )
    .await
}

const USER_1: &str = "00000000-0000-0000-0000-000000000011";
const CALLER_1: &str = "FINANCE:00000000-0000-0000-0000-000000000011@XC-B01";

// Row 1: FINANCE "Finance One" saves a limit -> "Finance One"
#[tokio::test]
async fn row_1_finance_saves_limit_answers_setter_name() {
    let db = TestDb::start().await;
    insert_user(&db, USER_1, "Finance One").await;

    let r = save(&db, CALLER_1, B01, json!("5000.00")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["updatedByName"], "Finance One", "{}", r.text);
    assert_eq!(r.body["updatedBy"], USER_1, "{}", r.text);
    assert_eq!(r.body["countDifferenceLimitThb"], "5000.00", "{}", r.text);

    db.finish().await;
}

// Row 2: then the view reads it -> "Finance One" and the view answers the restated shape
#[tokio::test]
async fn row_2_then_the_view_reads_it() {
    let db = TestDb::start().await;
    insert_user(&db, USER_1, "Finance One").await;

    let s = save(&db, CALLER_1, B01, json!("5000.00")).await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);

    let r = read(&db, CALLER_1, B01).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["data"]["updatedByName"], "Finance One", "{}", r.text);
    assert_eq!(r.body["data"]["updatedBy"], USER_1, "{}", r.text);
    assert_eq!(r.body["data"]["countDifferenceLimitThb"], "5000.00", "{}", r.text);
    assert_eq!(r.body["data"], s.body, "save and read must answer identical data");

    db.finish().await;
}

// Row 3: a branch with no row -> null (the restated pin)
#[tokio::test]
async fn row_3_a_branch_with_no_row() {
    let db = TestDb::start().await;

    let r = read(&db, CALLER_1, B01).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        r.body["data"],
        json!({
            "branch": B01,
            "countDifferenceLimitThb": null,
            "updatedBy": null,
            "updatedByName": null,
            "updatedAt": null,
        }),
        "{}",
        r.text
    );

    db.finish().await;
}

// Row 4: the setter's users.name is blank/empty/whitespace -> null
#[tokio::test]
async fn row_4_setter_name_is_blank_answers_null() {
    let db = TestDb::start().await;
    const USER_BLANK: &str = "00000000-0000-0000-0000-000000000014";
    const CALLER_BLANK: &str = "FINANCE:00000000-0000-0000-0000-000000000014@XC-B01";
    insert_user(&db, USER_BLANK, "   ").await;

    let s = save(&db, CALLER_BLANK, B01, json!("2000.00")).await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);
    assert_eq!(s.body["updatedByName"], Value::Null, "{}", s.text);
    assert_eq!(s.body["updatedBy"], USER_BLANK, "{}", s.text);

    let r = read(&db, CALLER_BLANK, B01).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["data"]["updatedByName"], Value::Null, "{}", r.text);
    assert_eq!(r.body["data"]["updatedBy"], USER_BLANK, "{}", r.text);

    db.finish().await;
}

// Row 5: updated_by names no users row -> null ; updatedBy still the setter's id
#[tokio::test]
async fn row_5_updated_by_names_no_users_row_answers_null() {
    let db = TestDb::start().await;
    const CALLER_UNKNOWN: &str = "FINANCE:00000000-0000-0000-0000-000000000099@XC-B01";
    const USER_UNKNOWN: &str = "00000000-0000-0000-0000-000000000099";

    let s = save(&db, CALLER_UNKNOWN, B01, json!("1000.00")).await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);
    assert_eq!(s.body["updatedByName"], Value::Null, "{}", s.text);
    assert_eq!(s.body["updatedBy"], USER_UNKNOWN, "{}", s.text);

    let r = read(&db, CALLER_UNKNOWN, B01).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["data"]["updatedByName"], Value::Null, "{}", r.text);
    assert_eq!(r.body["data"]["updatedBy"], USER_UNKNOWN, "{}", r.text);

    db.finish().await;
}
