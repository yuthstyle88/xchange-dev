//! Ticket #3400: admin_users view search query filter `q`.
//!
//! Behaviour table from issue #3400 §9:
//! - q=som with users "Somchai" and "Anan" -> only Somchai
//! - q=E0042, a user whose employee_code is E0042 -> that user
//! - q=SOM (case) -> Somchai
//! - q=% -> only names or codes that contain a literal %
//! - q absent / empty -> every user, the same rows as today
//! - q of 101 characters -> 400 invalidRequest, field q
//! - 3 matching users, limit=2, then before=<2nd id> with the same q -> pages of 2 then 1, no repeat, no skip
//! - another organisation's user who matches -> never listed

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
use chrono::{DateTime, Utc};
use serde_json::Value;
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
const B01: &str = "XC-B01";
const ADMIN: &str = "ADMIN@*";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
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
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("adminuserssearch");
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

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
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

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
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
    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn insert_user(db: &TestDb, org_id: &str, name: &str, employee_code: Option<&str>) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, employee_code, status, created_by) \
         VALUES ($1::uuid, $2, $3, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(org_id)
    .bind(name)
    .bind(employee_code)
    .fetch_one(&db.pool)
    .await
    .expect("insert user")
}

// ------------------------------------------------------------ behaviour table tests

/// Case 1 & Case 3: q=som with users "Somchai" and "Anan" -> only Somchai; q=SOM (case) -> Somchai
#[actix_web::test]
async fn test_q_case_insensitive_name_search() {
    let db = TestDb::start().await;
    let org_id = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let somchai = insert_user(&db, org_id, "Somchai", None).await;
    let _anan = insert_user(&db, org_id, "Anan", None).await;

    // q=som -> only Somchai
    let r1 = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=som"),
    )
    .await;
    assert_eq!(r1.status, StatusCode::OK, "{}", r1.text);
    let rows1 = r1.body["data"].as_array().expect("rows array");
    assert_eq!(
        rows1.len(),
        1,
        "q=som must return only Somchai: {:?}",
        rows1
    );
    assert_eq!(rows1[0]["userId"], somchai);
    assert_eq!(rows1[0]["name"], "Somchai");

    // q=SOM -> Somchai (case-insensitive)
    let r2 = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=SOM"),
    )
    .await;
    assert_eq!(r2.status, StatusCode::OK, "{}", r2.text);
    let rows2 = r2.body["data"].as_array().expect("rows array");
    assert_eq!(rows2.len(), 1, "q=SOM must return Somchai: {:?}", rows2);
    assert_eq!(rows2[0]["userId"], somchai);

    db.finish().await;
}

/// Case 2: q=E0042, a user whose employee_code is E0042 -> that user
#[actix_web::test]
async fn test_q_employee_code_search() {
    let db = TestDb::start().await;
    let org_id = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let u1 = insert_user(&db, org_id, "Kittisak", Some("E0042")).await;
    let _u2 = insert_user(&db, org_id, "Prasert", Some("E0043")).await;

    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=E0042"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1, "q=E0042 must return that user: {:?}", rows);
    assert_eq!(rows[0]["userId"], u1);
    assert_eq!(rows[0]["employeeCode"], "E0042");

    db.finish().await;
}

/// Case 4: q=% -> only names or codes that contain a literal %
#[actix_web::test]
async fn test_q_literal_percent_search() {
    let db = TestDb::start().await;
    let org_id = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let u_pct = insert_user(&db, org_id, "Discount 100% User", None).await;
    let _u_normal = insert_user(&db, org_id, "Normal User", None).await;

    // q=% URL-encoded is %25
    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=%25"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        1,
        "q=% must return only user containing literal %: {:?}",
        rows
    );
    assert_eq!(rows[0]["userId"], u_pct);
    assert_eq!(rows[0]["name"], "Discount 100% User");

    db.finish().await;
}

/// Case 5: q absent / empty -> every user, the same rows as today
#[actix_web::test]
async fn test_q_absent_or_empty_returns_every_user() {
    let db = TestDb::start().await;
    let org_id = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let u1 = insert_user(&db, org_id, "Alpha", None).await;
    let u2 = insert_user(&db, org_id, "Beta", None).await;

    // q absent
    let r_absent = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}"),
    )
    .await;
    assert_eq!(r_absent.status, StatusCode::OK, "{}", r_absent.text);
    let rows_absent = r_absent.body["data"].as_array().expect("rows array");
    assert_eq!(rows_absent.len(), 2);
    assert_eq!(rows_absent[0]["userId"], u1);
    assert_eq!(rows_absent[1]["userId"], u2);

    // q empty
    let r_empty = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q="),
    )
    .await;
    assert_eq!(r_empty.status, StatusCode::OK, "{}", r_empty.text);
    let rows_empty = r_empty.body["data"].as_array().expect("rows array");
    assert_eq!(rows_empty.len(), 2);
    assert_eq!(rows_empty[0]["userId"], u1);
    assert_eq!(rows_empty[1]["userId"], u2);

    db.finish().await;
}

/// Case 6: q of 101 characters -> 400 invalidRequest, field q
#[actix_web::test]
async fn test_q_over_100_characters_returns_400_invalid_request() {
    let db = TestDb::start().await;

    let long_q = "a".repeat(101);
    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q={long_q}"),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    assert_eq!(r.body["error"]["code"], "invalidRequest");
    assert_eq!(r.body["error"]["field"], "q");

    db.finish().await;
}

/// Case 7: 3 matching users, limit=2, then before=<2nd id> with the same q -> pages of 2 then 1, no repeat, no skip
#[actix_web::test]
async fn test_pagination_with_q_and_limit_and_before() {
    let db = TestDb::start().await;
    let org_id = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let u1 = insert_user(&db, org_id, "Tech Alice", None).await;
    let u2 = insert_user(&db, org_id, "Tech Bob", None).await;
    let u3 = insert_user(&db, org_id, "Tech Charlie", None).await;
    let _other = insert_user(&db, org_id, "Other User", None).await;

    // Page 1: limit=2&q=Tech
    let r1 = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=Tech&limit=2"),
    )
    .await;
    assert_eq!(r1.status, StatusCode::OK, "{}", r1.text);
    let rows1 = r1.body["data"].as_array().expect("rows array");
    assert_eq!(rows1.len(), 2, "page 1 must have 2 rows: {:?}", rows1);
    assert_eq!(rows1[0]["userId"], u1);
    assert_eq!(rows1[1]["userId"], u2);

    // Page 2: limit=2&q=Tech&before=<2nd id>
    let cursor = &u2;
    let r2 = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=Tech&limit=2&before={cursor}"),
    )
    .await;
    assert_eq!(r2.status, StatusCode::OK, "{}", r2.text);
    let rows2 = r2.body["data"].as_array().expect("rows array");
    assert_eq!(rows2.len(), 1, "page 2 must have 1 row: {:?}", rows2);
    assert_eq!(rows2[0]["userId"], u3);

    db.finish().await;
}

/// Case 8: another organisation's user who matches -> never listed
#[actix_web::test]
async fn test_another_org_matching_user_never_listed() {
    let db = TestDb::start().await;
    let org_id = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let own_user = insert_user(&db, org_id, "Somchai Own", None).await;

    // Insert a second organisation and a matching user in that organisation
    let other_org_id = "00000000-0000-0000-0000-0000000000b2";
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'YY', NULL, 'Asia/Bangkok', now())",
    )
    .bind(other_org_id)
    .execute(&db.pool)
    .await
    .expect("insert second org");

    let _foreign_user = insert_user(&db, other_org_id, "Somchai Foreign", None).await;

    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&q=Somchai"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        1,
        "only own org user must be returned: {:?}",
        rows
    );
    assert_eq!(rows[0]["userId"], own_user);
    assert_eq!(rows[0]["name"], "Somchai Own");

    db.finish().await;
}
