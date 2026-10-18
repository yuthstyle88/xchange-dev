//! RBAC f21 (F10, #3204): `admin_users` view answers employee profile columns.
//!
//! Behaviour table:
//! - a user with all four fields set -> the row carries the four values
//! - a user with none set -> the four fields are null
//! - paging with before -> unchanged (same users, same order)
//! - generated admin_users.ts is updated through export_types

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
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG_ADMIN: &str = "ADMIN@*";

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
        let schema = unique("rbac_f10");
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

fn legacy_of(spec: &str) -> Sessions {
    let first = spec.split(['+', '#']).next().unwrap_or("");
    match first.split_once('@') {
        None => Sessions {
            owner: None,
            staff: None,
        },
        Some((_, "*")) => Sessions {
            owner: Some(Actor::owner(user_id(spec))),
            staff: None,
        },
        Some((_, branch)) => Sessions {
            owner: None,
            staff: Actor::staff(user_id(spec), OldRole::Finance, BranchId::new(branch)),
        },
    }
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let s = legacy_of(&spec);
            let user = user_of(&spec);
            Callers {
                owner: s.owner,
                staff: s.staff,
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

async fn a_user_with_name(db: &TestDb, name: &str, status: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT organization_id, $1, $2, 'test' FROM branches WHERE id = $3 \
         RETURNING id::text",
    )
    .bind(name)
    .bind(status)
    .bind(B01)
    .fetch_one(&db.pool)
    .await
    .expect("insert user")
}

async fn admin_users(db: &TestDb, who: &str, query: &str) -> Answer {
    get(
        db,
        who,
        &format!("/api/v3/views/admin_users?branch={B01}&{query}"),
    )
    .await
}

// ------------------------------------------------------------ Behaviour table tests

#[test]
fn update_generated_admin_users_ts() {
    let scratch = std::env::temp_dir().join(format!("rbac_f10_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("create scratch dir");
    let view = xchange::presentation::http::v3::VIEWS
        .iter()
        .find(|v| v.name == "admin_users")
        .expect("admin_users view");
    (view.export_types)(&scratch).expect("export types");
    let produced = std::fs::read_to_string(scratch.join("admin_users.ts"))
        .expect("read produced admin_users.ts");
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("web/src/shared/api/generated/admin_users.ts");
    let current = std::fs::read_to_string(&target).unwrap_or_default();
    if current != produced {
        std::fs::write(&target, &produced).expect("update admin_users.ts");
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert_eq!(
        std::fs::read_to_string(&target).expect("read target"),
        produced
    );
}

/// Row 1: a user with all four fields set -> the row carries the four values
#[actix_web::test]
async fn user_with_all_four_fields_set_returns_values() {
    let db = TestDb::start().await;
    let user = a_user_with_name(&db, &unique("u_full"), "ACTIVE").await;
    sqlx::query(
        "UPDATE users SET \
         employee_code = 'EMP-900', \
         department = 'Finance Dept', \
         position = 'Senior Accountant', \
         tell_code = 'TC-88' \
         WHERE id = $1::uuid",
    )
    .bind(&user)
    .execute(&db.pool)
    .await
    .expect("update profile");

    let a = admin_users(&db, ORG_ADMIN, "").await;
    assert_eq!(a.status, StatusCode::OK, "read admin_users: {}", a.text);

    let rows = a.body["data"].as_array().expect("data array");
    let row = rows
        .iter()
        .find(|r| r["userId"] == user)
        .expect("found row");

    assert_eq!(row["employeeCode"], "EMP-900");
    assert_eq!(row["department"], "Finance Dept");
    assert_eq!(row["position"], "Senior Accountant");
    assert_eq!(row["tellCode"], "TC-88");

    db.finish().await;
}

/// Row 2: a user with none set -> the four fields are null
#[actix_web::test]
async fn user_with_none_set_returns_nulls() {
    let db = TestDb::start().await;
    let user = a_user_with_name(&db, &unique("u_empty"), "ACTIVE").await;

    let a = admin_users(&db, ORG_ADMIN, "").await;
    assert_eq!(a.status, StatusCode::OK, "read admin_users: {}", a.text);

    let rows = a.body["data"].as_array().expect("data array");
    let row = rows
        .iter()
        .find(|r| r["userId"] == user)
        .expect("found row");

    assert!(row["employeeCode"].is_null(), "employeeCode is null");
    assert!(row["department"].is_null(), "department is null");
    assert!(row["position"].is_null(), "position is null");
    assert!(row["tellCode"].is_null(), "tellCode is null");

    db.finish().await;
}

/// Row 3: paging with before -> unchanged (same users, same order)
#[actix_web::test]
async fn paging_with_before_is_unchanged() {
    let db = TestDb::start().await;
    let prefix = unique("sort");
    let u1 = a_user_with_name(&db, &format!("{prefix}_A"), "ACTIVE").await;
    let u2 = a_user_with_name(&db, &format!("{prefix}_B"), "ACTIVE").await;
    let u3 = a_user_with_name(&db, &format!("{prefix}_C"), "ACTIVE").await;

    // Page 1: limit 2
    let a1 = admin_users(&db, ORG_ADMIN, "limit=2").await;
    assert_eq!(a1.status, StatusCode::OK, "{}", a1.text);
    let rows1 = a1.body["data"].as_array().expect("rows1");
    assert_eq!(rows1.len(), 2);
    assert_eq!(rows1[0]["userId"], u1);
    assert_eq!(rows1[1]["userId"], u2);

    // Page 2: before = u2
    let a2 = admin_users(&db, ORG_ADMIN, &format!("before={u2}&limit=2")).await;
    assert_eq!(a2.status, StatusCode::OK, "{}", a2.text);
    let rows2 = a2.body["data"].as_array().expect("rows2");
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0]["userId"], u3);

    db.finish().await;
}
