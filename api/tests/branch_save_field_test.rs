//! #4921 (server half): `branchSave` (`PUT /api/branches/profile`) puts a timezone or opening-hours refusal at its
//! field, and the branch tablets' `add` answers an over-long name with its own sentence, not "enter a device name".
//!
//! Written before the implementation by the ticket author's lane (claude/b873aa20, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Behaviour rows pinned (ticket §3; the screen rows BR4–BR6 are #4939's):
//! - BR1 save timezone `Asia/Bangkk` ⇒ 400 `validationFailed`, field `timezone`.
//! - BR2 save an open day (`mon`) with close `""` ⇒ 400 `validationFailed`, field `hours.mon`.
//! - BR3 tablet name of 61 characters ⇒ 400 with a sentence of its own, not the empty-name one (which stays as today).
//!
//! Harness copied from `v3_org_admin_test.rs`. Needs `TEST_DATABASE_URL`; nothing skips.

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
const TABLETS: &str = "/api/branch-tablets";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("brfield");
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

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
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

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "ADMIN" => Role::Admin,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &format!("user-{}", spec.replace(['@', '*'], "-")),
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

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move { test_callers::callers(None, user_of(&spec)) })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn send(db: &TestDb, method: Method, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, ADMIN))
        .insert_header(("x-real-ip", "203.0.113.30"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn profile() -> Value {
    json!({
        "branch": B01,
        "code": "SILOM",
        "address": "1 Silom Rd",
        "timezone": "Asia/Bangkok",
        "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
        "bookingEnabled": true
    })
}

async fn refused_at(db: &TestDb, body: Value, field: &str, what: &str) {
    let a = send(db, Method::PUT, PROFILE, body).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["code"].as_str(),
        Some("validationFailed"),
        "{what}: code; body {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"].as_str(),
        Some(field),
        "{what}: field; body {}",
        a.text
    );
    assert_eq!(
        db.count("SELECT count(*) FROM branches WHERE id = 'XC-B01' AND code = 'SILOM'")
            .await,
        0,
        "{what}: nothing saved"
    );
}

#[actix_web::test]
async fn br1_a_misspelt_timezone_is_refused_at_timezone() {
    let db = TestDb::start().await;
    let mut body = profile();
    body["timezone"] = json!("Asia/Bangkk");
    refused_at(&db, body, "timezone", "BR1 Asia/Bangkk").await;
    db.finish().await;
}

#[actix_web::test]
async fn br2_an_open_day_with_a_cleared_close_is_refused_at_hours_day() {
    let db = TestDb::start().await;
    let mut body = profile();
    body["openingHours"] = json!({ "mon": [{ "open": "09:00", "close": "" }] });
    refused_at(&db, body, "hours.mon", "BR2 mon close \"\"").await;
    db.finish().await;
}

#[actix_web::test]
async fn br3_a_tablet_name_of_61_characters_has_its_own_sentence() {
    let db = TestDb::start().await;
    let add =
        |name: String| json!({ "action": "add", "payload": { "branchId": B01, "name": name } });

    let empty = send(&db, Method::POST, TABLETS, add("   ".to_owned())).await;
    assert_eq!(
        empty.status,
        StatusCode::BAD_REQUEST,
        "empty: {}",
        empty.text
    );
    let empty_sentence = empty.body["error"].as_str().unwrap_or("").to_owned();
    assert_eq!(
        empty_sentence, "ระบุชื่อเครื่อง",
        "the empty name is told to enter a name, as today"
    );

    let long = send(&db, Method::POST, TABLETS, add("ก".repeat(61))).await;
    assert_eq!(
        long.status,
        StatusCode::BAD_REQUEST,
        "BR3 61 units: {}",
        long.text
    );
    let long_sentence = long.body["error"].as_str().unwrap_or("");
    assert!(
        !long_sentence.is_empty() && long_sentence != empty_sentence,
        "BR3: a too-long name has its own sentence, not the empty-name one; body {}",
        long.text
    );
    assert_eq!(
        db.count("SELECT count(*) FROM branch_tablets").await,
        0,
        "BR3: no tablet added"
    );
    db.finish().await;
}
