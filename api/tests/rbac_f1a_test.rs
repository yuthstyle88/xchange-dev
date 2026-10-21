//! RBAC f12 (F1a, #3148): `POST /api/users/suspend` (`userSuspend`).
//!
//! Behaviour table rows:
//! 1. ADMIN suspends an ACTIVE user who holds no cash -> 200, status: "SUSPENDED", version +1
//! 2. the user holds cash (custody_holds() answers true) -> 409 holdsCash (ignored pending C3 #3147)
//! 3. suspend a PENDING user -> 409 statusNotAllowed
//! 4. suspend the last owner -> 409 lastOwner
//! 5. a user of another organisation -> 403 outOfScope (same refusal user_disable gives)
//! 6. no UsersManage (a TELLER) -> 403, policy refusal (roleNotAllowedHere)
//! 7. unknown userId -> 404 notFound (same refusal user_disable gives)

#![allow(dead_code)]

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
use xc_testdb::people::{RoleRow, seed_roles};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG_ADMIN: &str = "ADMIN@*";
const OWNER: &str = "EXECUTIVE@*+ADMIN@*";

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
        let schema = unique("rbac_f1a");
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

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn version(&self, user: &str) -> i64 {
        self.count(&format!(
            "SELECT version::bigint FROM users WHERE id = '{user}'::uuid"
        ))
        .await
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

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key<'_>,
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

async fn post(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Value,
    key: Key<'_>,
) -> Answer {
    call(db, who, method, uri, Some(body), key).await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(a.status, status, "{what}: {}", a.text);
    assert_eq!(code(a), want, "{what}: {}", a.text);
}

async fn a_user(db: &TestDb, status: &str, roles: &[(&str, Option<&str>)]) -> String {
    let name = unique("u");
    let user = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, '{name}', '{status}', 'test' FROM branches WHERE id = '{B01}' \
             RETURNING id::text"
        ))
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
    .expect("seed user roles");
    user
}

async fn status_of(db: &TestDb, user: &str) -> String {
    db.text(&format!(
        "SELECT status FROM users WHERE id = '{user}'::uuid"
    ))
    .await
    .unwrap_or_default()
}

async fn suspend(db: &TestDb, who: &str, user: &str) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/suspend",
        json!({ "userId": user }),
        Key::Fresh,
    )
    .await
}

async fn disable(db: &TestDb, who: &str, user: &str) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/disable",
        json!({ "userId": user }),
        Key::Fresh,
    )
    .await
}

// ------------------------------------------------------------ Behaviour table tests

/// Row 1: ADMIN suspends an ACTIVE user who holds no cash -> 200, status: "SUSPENDED", version +1
#[actix_web::test]
async fn admin_suspends_an_active_user_who_holds_no_cash() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let v0 = db.version(&user).await;

    let a = suspend(&db, ORG_ADMIN, &user).await;
    assert_eq!(a.status, StatusCode::OK, "suspend active user: {}", a.text);
    assert_eq!(a.body["userId"], user);
    assert_eq!(a.body["status"], "SUSPENDED");
    assert_eq!(a.body["version"], v0 + 1);

    assert_eq!(status_of(&db, &user).await, "SUSPENDED");
    assert_eq!(db.version(&user).await, v0 + 1);

    db.finish().await;
}

/// Row 2: the user holds cash (custody_holds() answers true) -> 409 holdsCash
/// Marked ignore until RBAC f11 #3147 (C3) merges into feat/rbac.
#[actix_web::test]
async fn a_user_holding_cash_cannot_be_suspended() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ('shift-holds-cash', $1, 'OPEN', now(), 'test', 'open-req-1', $2::uuid)",
    )
    .bind(B01)
    .bind(&user)
    .execute(&db.pool)
    .await
    .expect("seed open shift");

    let a = suspend(&db, ORG_ADMIN, &user).await;
    refused(&a, StatusCode::CONFLICT, "holdsCash", "user holds cash");
    assert_eq!(status_of(&db, &user).await, "ACTIVE");

    db.finish().await;
}

/// Row 3: suspend a PENDING user -> 409 statusNotAllowed
#[actix_web::test]
async fn suspend_a_pending_user_is_refused() {
    let db = TestDb::start().await;
    let user = a_user(&db, "PENDING", &[("TELLER", Some(B01))]).await;
    let v0 = db.version(&user).await;

    let a = suspend(&db, ORG_ADMIN, &user).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "statusNotAllowed",
        "suspend a PENDING user",
    );
    assert_eq!(status_of(&db, &user).await, "PENDING");
    assert_eq!(db.version(&user).await, v0);

    db.finish().await;
}

/// Row 4: suspend the last owner -> 409 lastOwner
#[actix_web::test]
async fn suspend_the_last_owner_is_refused() {
    let db = TestDb::start().await;
    let only = a_user(&db, "ACTIVE", &[("EXECUTIVE", None), ("ADMIN", None)]).await;
    let v0 = db.version(&only).await;

    let a = suspend(&db, OWNER, &only).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "lastOwner",
        "suspend the only owner",
    );
    assert_eq!(status_of(&db, &only).await, "ACTIVE");
    assert_eq!(db.version(&only).await, v0);

    db.finish().await;
}

/// Row 5: a user of another organisation -> the same refusal user_disable gives for it
#[actix_web::test]
async fn a_user_of_another_organisation_is_refused() {
    let db = TestDb::start().await;
    let other_org = "00000000-0000-0000-0000-0000000000f3";
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) VALUES ($1::uuid, 'XC3', 'Asia/Bangkok', now())",
    )
    .bind(other_org)
    .execute(&db.pool)
    .await
    .expect("another organisation");

    let foreign: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Foreign user', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(other_org)
    .fetch_one(&db.pool)
    .await
    .expect("a user of another organisation");

    let a = suspend(&db, ORG_ADMIN, &foreign).await;
    let d = disable(&db, ORG_ADMIN, &foreign).await;

    assert_eq!(a.status, d.status, "same status as user_disable");
    assert_eq!(code(&a), code(&d), "same refusal code as user_disable");
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "foreign user suspend",
    );

    db.finish().await;
}

/// Row 6: no UsersManage (a TELLER) -> 403 / 400, the policy refusal
#[actix_web::test]
async fn no_users_manage_teller_is_refused() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;
    let v0 = db.version(&user).await;

    let a = suspend(&db, "TELLER@XC-B01", &user).await;
    let d = disable(&db, "TELLER@XC-B01", &user).await;

    assert_eq!(
        a.status, d.status,
        "same status as user_disable: {}",
        a.text
    );
    assert_eq!(code(&a), "roleNotAllowedHere", "policy refusal: {}", a.text);
    assert_eq!(code(&a), code(&d), "same refusal code as user_disable");
    assert_eq!(status_of(&db, &user).await, "ACTIVE");
    assert_eq!(db.version(&user).await, v0);

    db.finish().await;
}

/// Row 7: unknown userId -> the user_in_organisation refusal (as user_disable)
#[actix_web::test]
async fn unknown_user_id_is_refused() {
    let db = TestDb::start().await;
    let missing = "00000000-0000-0000-0000-000000000099";

    let a = suspend(&db, ORG_ADMIN, missing).await;
    let d = disable(&db, ORG_ADMIN, missing).await;

    assert_eq!(a.status, d.status, "same status as user_disable");
    assert_eq!(code(&a), code(&d), "same refusal code as user_disable");
    refused(&a, StatusCode::NOT_FOUND, "notFound", "missing user");

    db.finish().await;
}
