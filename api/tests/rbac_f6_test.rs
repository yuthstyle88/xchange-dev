//! RBAC f18 (F6, #3154): `GET /api/v3/views/admin_employee_register` (`adminEmployeeRegister`).
//!
//! Behaviour table rows:
//! 1. two users, one with two live roles, one with none -> three rows: two for the first, one with null role for the second
//! 2. an ended role (valid_to in the past) -> not shown
//! 3. the profile columns set -> shown
//! 4. a user of another organisation -> not shown
//! 5. an ADMIN (UsersManage) -> 200
//! 6. a caller without the permission -> 403 / 400 policy refusal

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
use chrono::{DateTime, Duration as CDuration, Utc};
use serde_json::Value;
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
const ORG_ADMIN: &str = "ADMIN@*";

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
        let schema = unique("rbac_f6");
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

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    call(db, who, Method::GET, uri, None, Key::Fresh).await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

type RoleSpec<'a> = (
    &'a str,
    Option<&'a str>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

async fn a_user_full(
    db: &TestDb,
    status: &str,
    employee_code: Option<&str>,
    position: Option<&str>,
    department: Option<&str>,
    tell_code: Option<&str>,
    roles: &[RoleSpec<'_>],
) -> String {
    let name = unique("u");
    let user: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, employee_code, position, department, tell_code, created_by) \
         SELECT organization_id, $1, $2, $3, $4, $5, $6, 'test' FROM branches WHERE id = $7 \
         RETURNING id::text",
    )
    .bind(&name)
    .bind(status)
    .bind(employee_code)
    .bind(position)
    .bind(department)
    .bind(tell_code)
    .bind(B01)
    .fetch_one(&db.pool)
    .await
    .expect("insert user");

    for (role, branch, from, to) in roles {
        let from_ts = from.unwrap_or_else(|| Utc::now() - CDuration::days(1));
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
             SELECT $1::uuid, $2, organization_id, $3, $4, $5 FROM branches WHERE id = $6",
        )
        .bind(&user)
        .bind(role)
        .bind(branch)
        .bind(from_ts)
        .bind(to)
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("insert role");
    }

    user
}

async fn read_register(db: &TestDb, who: &str, branch: &str) -> Answer {
    get(
        db,
        who,
        &format!("/api/v3/views/admin_employee_register?branch={branch}"),
    )
    .await
}

// ------------------------------------------------------------ Behaviour table tests

/// Row 1: two users, one with two live roles, one with none -> three rows: two for the first, one with null role for the second
#[actix_web::test]
async fn two_users_one_with_two_live_roles_one_with_none() {
    let db = TestDb::start().await;
    let u1 = a_user_full(
        &db,
        "ACTIVE",
        Some("EMP-01"),
        None,
        None,
        None,
        &[
            ("TELLER", Some(B01), None, None),
            ("FINANCE", Some(B01), None, None),
        ],
    )
    .await;

    let u2 = a_user_full(&db, "ACTIVE", Some("EMP-02"), None, None, None, &[]).await;

    let a = read_register(&db, ORG_ADMIN, B01).await;
    assert_eq!(a.status, StatusCode::OK, "read register: {}", a.text);

    let rows = a.body["data"].as_array().expect("data array");
    let u1_rows: Vec<&Value> = rows.iter().filter(|r| r["userId"] == u1).collect();
    let u2_rows: Vec<&Value> = rows.iter().filter(|r| r["userId"] == u2).collect();

    assert_eq!(u1_rows.len(), 2, "user 1 has 2 rows");
    let roles: Vec<Option<&str>> = u1_rows.iter().map(|r| r["role"].as_str()).collect();
    assert!(roles.contains(&Some("TELLER")));
    assert!(roles.contains(&Some("FINANCE")));

    assert_eq!(u2_rows.len(), 1, "user 2 has 1 row");
    assert!(u2_rows[0]["role"].is_null(), "user 2 role is null");
    assert!(u2_rows[0]["branch"].is_null(), "user 2 branch is null");

    db.finish().await;
}

/// Row 2: an ended role (valid_to in the past) -> not shown
#[actix_web::test]
async fn an_ended_role_is_not_shown() {
    let db = TestDb::start().await;
    let past = Utc::now() - CDuration::hours(2);
    let u = a_user_full(
        &db,
        "ACTIVE",
        Some("EMP-ENDED"),
        None,
        None,
        None,
        &[
            ("TELLER", Some(B01), None, None),
            ("BRANCH_MANAGER", Some(B01), None, Some(past)),
        ],
    )
    .await;

    let a = read_register(&db, ORG_ADMIN, B01).await;
    assert_eq!(a.status, StatusCode::OK, "read register: {}", a.text);

    let rows = a.body["data"].as_array().expect("data array");
    let u_rows: Vec<&Value> = rows.iter().filter(|r| r["userId"] == u).collect();

    assert_eq!(u_rows.len(), 1, "only 1 live role shown");
    assert_eq!(u_rows[0]["role"], "TELLER");

    db.finish().await;
}

/// Row 3: the profile columns set -> shown
#[actix_web::test]
async fn profile_columns_are_shown() {
    let db = TestDb::start().await;
    let u = a_user_full(
        &db,
        "ACTIVE",
        Some("EMP-V9"),
        Some("Senior Analyst"),
        Some("Operations"),
        Some("TC-007"),
        &[("TELLER", Some(B01), None, None)],
    )
    .await;

    let a = read_register(&db, ORG_ADMIN, B01).await;
    assert_eq!(a.status, StatusCode::OK, "read register: {}", a.text);

    let rows = a.body["data"].as_array().expect("data array");
    let row = rows.iter().find(|r| r["userId"] == u).expect("found row");

    assert_eq!(row["employeeCode"], "EMP-V9");
    assert_eq!(row["position"], "Senior Analyst");
    assert_eq!(row["department"], "Operations");
    assert_eq!(row["tellCode"], "TC-007");
    assert_eq!(row["role"], "TELLER");
    assert_eq!(row["status"], "ACTIVE");

    db.finish().await;
}

/// Row 4: a user of another organisation -> not shown
#[actix_web::test]
async fn a_user_of_another_organisation_is_not_shown() {
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
        "INSERT INTO users (organization_id, name, status, employee_code, created_by) \
         VALUES ($1::uuid, 'Foreign User', 'ACTIVE', 'FOR-01', 'test') RETURNING id::text",
    )
    .bind(other_org)
    .fetch_one(&db.pool)
    .await
    .expect("foreign user");

    let a = read_register(&db, ORG_ADMIN, B01).await;
    assert_eq!(a.status, StatusCode::OK, "read register: {}", a.text);

    let rows = a.body["data"].as_array().expect("data array");
    assert!(
        rows.iter().all(|r| r["userId"] != foreign),
        "foreign user must not be present"
    );

    db.finish().await;
}

/// Row 5: an ADMIN (UsersManage) -> 200
#[actix_web::test]
async fn an_admin_reads_200() {
    let db = TestDb::start().await;
    let a = read_register(&db, ORG_ADMIN, B01).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["changed"], true);
    assert!(a.body["data"].is_array());
    db.finish().await;
}

/// Row 6: a caller without the permission -> 403 / 400 policy refusal
#[actix_web::test]
async fn a_caller_without_permission_is_refused() {
    let db = TestDb::start().await;
    let a = read_register(&db, "TELLER@XC-B01", B01).await;
    assert_ne!(a.status, StatusCode::OK, "teller read: {}", a.text);
    assert_eq!(code(&a), "roleNotAllowedHere", "policy refusal: {}", a.text);
    db.finish().await;
}
