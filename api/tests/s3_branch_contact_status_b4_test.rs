//! S3 (#3406, controller ruling 1) B4 test: a `branchSave` without `contactChannels` and `status` keeps the stored
//! values.

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
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ADMIN: &str = "ADMIN@*";
const PROFILE: &str = "/api/branches/profile";

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
        let schema = unique("s3b4");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert the branch");
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

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "ADMIN" => Role::Admin,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
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
        Box::pin(async move { test_callers::callers(None, user_of(&spec)) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
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

/// B4 (#3406 controller ruling 1): a save without `contactChannels` and `status` keeps the stored values.
#[actix_web::test]
async fn s3_b4_a_save_without_channels_or_status_keeps_the_stored_values() {
    let db = TestDb::start().await;
    let channels = json!([
        { "kind": "PHONE", "value": "+66 2 000 0000" },
        { "kind": "LINE", "value": "@xchange" }
    ]);
    let first = call(
        &db,
        ADMIN,
        Method::PUT,
        PROFILE,
        Some(json!({
            "branch": B01,
            "code": "SILOM",
            "address": "1 Silom Rd",
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": true,
            "contactChannels": channels,
            "status": "INACTIVE"
        })),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    let second = call(
        &db,
        ADMIN,
        Method::PUT,
        PROFILE,
        Some(json!({
            "branch": B01,
            "code": "SILOM2",
            "address": "2 Silom Rd",
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": false
        })),
    )
    .await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert_eq!(
        second.body["contactChannels"], channels,
        "B4 save answer keeps contactChannels: {}",
        second.text
    );
    assert_eq!(
        second.body["status"],
        json!("INACTIVE"),
        "B4 save answer keeps status: {}",
        second.text
    );

    let view = call(
        &db,
        ADMIN,
        Method::GET,
        &format!("/api/v3/views/admin_branches_profile?branch={B01}"),
        None,
    )
    .await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.text);
    let data = &view.body["data"];
    assert_eq!(data["code"], json!("SILOM2"), "B4 updates code: {data}");
    assert_eq!(
        data["contactChannels"], channels,
        "B4 view keeps contactChannels: {data}"
    );
    assert_eq!(
        data["status"],
        json!("INACTIVE"),
        "B4 view keeps status: {data}"
    );
    db.finish().await;
}
