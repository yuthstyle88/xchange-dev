//! RBAC f26 (#3247): audit rows record client IP from CommandEndpoint task-local.
//!
//! Behaviour table:
//! - userSuspend with x-real-ip: 203.0.113.5 -> user_audit.ip = 203.0.113.5 (read with host(ip))
//! - userGrant with x-real-ip: 203.0.113.5 -> user_audit.ip = 203.0.113.5
//! - userProfileSave with x-real-ip: 203.0.113.5 -> user_audit.ip = 203.0.113.5
//! - a command with no header -> ip is NULL, never an error
//! - a command with invalid IP header -> ip is NULL, never fails the command
//! - a non-user v3 command (branchSave) -> unchanged behaviour

#![allow(dead_code)]

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_roles};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ADMIN: &str = "ADMIN@*";
const OWNER: &str = "EXECUTIVE@*+ADMIN@*";
const TEST_IP: &str = "203.0.113.5";

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
        let schema = unique("rbac_audit_ip");
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

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_id(spec: &str) -> String {
    match spec.split_once('#') {
        Some((_, id)) => id.to_owned(),
        None => format!("user-{}", spec.replace(['@', '*', '+'], "-")),
    }
}

fn user_of(spec: &str) -> Option<User> {
    let roles_part = spec.split_once('#').map_or(spec, |(r, _)| r);
    let mut roles = Vec::new();
    for one in roles_part.split('+') {
        let (role, branch) = one.split_once('@')?;
        let role = match role {
            "FINANCE" => Role::Finance,
            "ADMIN" => Role::Admin,
            "EXECUTIVE" => Role::Executive,
            "BRANCH_MANAGER" => Role::BranchManager,
            "TELLER" => Role::Teller,
            other => panic!("unknown role {other}"),
        };
        let branch = (branch != "*").then(|| BranchId::new(branch));
        roles.push(ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        ));
    }
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        roles,
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

enum Key<'a> {
    Fresh,
    This(&'a str),
}

async fn call_with_ip(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key<'_>,
    ip_header: Option<&str>,
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
    if let Some(ip) = ip_header {
        req = req.insert_header(("x-real-ip", ip));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
        match key {
            Key::Fresh => req = req.insert_header(("idempotency-key", unique("key"))),
            Key::This(k) => req = req.insert_header(("idempotency-key", k.to_owned())),
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn a_user(
    db: &TestDb,
    status: &str,
    auth_subject: Option<&str>,
    roles: &[(&str, Option<&str>)],
) -> String {
    let name = unique("u");
    let user: String = sqlx::query_scalar(AssertSqlSafe(format!(
        "INSERT INTO users (organization_id, auth_subject, name, status, created_by) \
         SELECT organization_id, {}, '{name}', '{status}', 'test' FROM branches WHERE id = '{B01}' \
         RETURNING id::text",
        auth_subject.map_or("NULL".to_owned(), |s| format!("'{s}'")),
    )))
    .fetch_one(&db.pool)
    .await
    .expect("a user of XC-B01's organisation");
    let rows: Vec<RoleRow<'_>> = roles
        .iter()
        .map(|&(role_code, branch_id)| RoleRow {
            role_code,
            branch_id,
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        })
        .collect();
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_roles(
        &mut conn,
        &user,
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        &rows,
    )
    .await
    .expect("insert user role");
    user
}

async fn latest_audit_ip(db: &TestDb, action: &str, user_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT host(ip) FROM user_audit WHERE action = $1 AND subject_id = $2 ORDER BY at DESC, id DESC LIMIT 1"
    )
    .bind(action)
    .bind(user_id)
    .fetch_optional(&db.pool)
    .await
    .unwrap_or_default()
    .flatten()
}

async fn audit_count(db: &TestDb, action: &str, user_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM user_audit WHERE action = $1 AND subject_id = $2",
    )
    .bind(action)
    .bind(user_id)
    .fetch_one(&db.pool)
    .await
    .unwrap_or(0)
}

// ------------------------------------------------------------ Behaviour table tests

#[actix_web::test]
async fn user_suspend_with_real_ip_header_records_ip_in_audit_row() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", None, &[]).await;

    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/suspend",
        Some(json!({ "userId": user })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "suspend failed: {}", ans.text);

    assert_eq!(audit_count(&db, "userSuspend", &user).await, 1);
    let ip = latest_audit_ip(&db, "userSuspend", &user).await;
    assert_eq!(ip.as_deref(), Some(TEST_IP));

    db.finish().await;
}

#[actix_web::test]
async fn user_grant_with_real_ip_header_records_ip_in_audit_row() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", None, &[]).await;

    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/grant",
        Some(json!({
            "userId": user,
            "role": "TELLER",
            "branch": B01,
        })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "grant failed: {}", ans.text);

    assert_eq!(audit_count(&db, "userGrant", &user).await, 1);
    let ip = latest_audit_ip(&db, "userGrant", &user).await;
    assert_eq!(ip.as_deref(), Some(TEST_IP));

    db.finish().await;
}

#[actix_web::test]
async fn user_profile_save_with_real_ip_header_records_ip_in_audit_row() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", None, &[]).await;

    let code = unique("EMP");
    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/profile",
        Some(json!({
            "userId": user,
            "employeeCode": code,
            "department": "Treasury",
            "position": "Officer",
            "tellCode": null,
        })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "profile save failed: {}",
        ans.text
    );

    assert_eq!(audit_count(&db, "userProfileSave", &user).await, 1);
    let ip = latest_audit_ip(&db, "userProfileSave", &user).await;
    assert_eq!(ip.as_deref(), Some(TEST_IP));

    db.finish().await;
}

#[actix_web::test]
async fn command_with_no_header_records_null_ip_without_error() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", None, &[]).await;

    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/suspend",
        Some(json!({ "userId": user })),
        Key::Fresh,
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "suspend failed: {}", ans.text);

    assert_eq!(audit_count(&db, "userSuspend", &user).await, 1);
    let ip = latest_audit_ip(&db, "userSuspend", &user).await;
    assert_eq!(ip, None, "ip should be NULL when no header provided");

    db.finish().await;
}

#[actix_web::test]
async fn command_with_invalid_ip_header_records_null_ip_and_never_fails() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", None, &[]).await;

    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/suspend",
        Some(json!({ "userId": user })),
        Key::Fresh,
        Some("not-an-ip-address"),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "suspend with invalid ip header failed: {}",
        ans.text
    );

    assert_eq!(audit_count(&db, "userSuspend", &user).await, 1);
    let ip = latest_audit_ip(&db, "userSuspend", &user).await;
    assert_eq!(ip, None, "non-IP string should be mapped to None / NULL");

    db.finish().await;
}

#[actix_web::test]
async fn non_user_v3_command_preserves_behaviour() {
    let db = TestDb::start().await;

    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::PUT,
        "/api/branches/profile",
        Some(json!({
            "branch": B01,
            "code": "B01-TEST",
            "address": "1 Silom Rd",
            "timezone": "Asia/Bangkok",
            "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
            "bookingEnabled": true
        })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "branch profile save failed: {}",
        ans.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn other_user_commands_record_ip_in_audit_row() {
    let db = TestDb::start().await;

    // userRevoke
    let user = a_user(&db, "ACTIVE", None, &[("TELLER", Some(B01))]).await;
    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/revoke",
        Some(json!({
            "userId": user,
            "role": "TELLER",
            "branch": B01,
        })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "revoke failed: {}", ans.text);
    assert_eq!(
        latest_audit_ip(&db, "userRevoke", &user).await.as_deref(),
        Some(TEST_IP)
    );

    // userDisable
    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/disable",
        Some(json!({ "userId": user })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "disable failed: {}", ans.text);
    assert_eq!(
        latest_audit_ip(&db, "userDisable", &user).await.as_deref(),
        Some(TEST_IP)
    );

    // userEnable (needs OWNER and bound user)
    let bound_user = a_user(
        &db,
        "DISABLED",
        Some("sub-bound-test"),
        &[("TELLER", Some(B01))],
    )
    .await;
    let ans = call_with_ip(
        &db,
        OWNER,
        Method::POST,
        "/api/users/enable",
        Some(json!({ "userId": bound_user })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "enable failed: {}", ans.text);
    assert_eq!(
        latest_audit_ip(&db, "userEnable", &bound_user)
            .await
            .as_deref(),
        Some(TEST_IP)
    );

    // userSuspend then userResume
    let active_user = a_user(&db, "ACTIVE", None, &[]).await;
    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/suspend",
        Some(json!({ "userId": active_user })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "suspend failed: {}", ans.text);

    let ans = call_with_ip(
        &db,
        ADMIN,
        Method::POST,
        "/api/users/resume",
        Some(json!({ "userId": active_user })),
        Key::Fresh,
        Some(TEST_IP),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "resume failed: {}", ans.text);
    assert_eq!(
        latest_audit_ip(&db, "userResume", &active_user)
            .await
            .as_deref(),
        Some(TEST_IP)
    );

    db.finish().await;
}
