//! RBAC f13 (F1b, #3149): `POST /api/users/resume` (`userResume`) HTTP tests.
//!
//! Behaviour table:
//! - ADMIN resumes a SUSPENDED user -> 200, status: "ACTIVE", version +1
//! - resume an ACTIVE user -> 409 statusNotAllowed
//! - resume a DISABLED user -> 409 statusNotAllowed
//! - a user of another organisation -> the same refusal user_disable gives for it (403 outOfScope)
//! - no UsersManage (a TELLER) -> 403, the policy refusal
//! - unknown userId -> the user_in_organisation refusal (as user_disable: 404 notFound)

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
use uuid::Uuid;
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
        let schema = unique("rbac_f1b");
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
    #[allow(dead_code)]
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

async fn allow_suspended(db: &TestDb) {
    sqlx::query("ALTER TABLE users DROP CONSTRAINT IF EXISTS users_status_check")
        .execute(&db.pool)
        .await
        .expect("drop the status check");
    sqlx::query(
        "ALTER TABLE users ADD CONSTRAINT users_status_check \
         CHECK (status IN ('PENDING', 'ACTIVE', 'SUSPENDED', 'DISABLED'))",
    )
    .execute(&db.pool)
    .await
    .expect("the widened status check");
}

async fn status_of(db: &TestDb, user: &str) -> String {
    db.text(&format!(
        "SELECT status FROM users WHERE id = '{user}'::uuid"
    ))
    .await
    .unwrap_or_default()
}

async fn resume(db: &TestDb, who: &str, user: &str) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/resume",
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

async fn enable(db: &TestDb, who: &str, user: &str) -> Answer {
    post(
        db,
        who,
        Method::POST,
        "/api/users/enable",
        json!({ "userId": user }),
        Key::Fresh,
    )
    .await
}

fn refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(a.status, status, "{what}: {}", a.text);
    assert_eq!(code(a), want, "{what}: {}", a.text);
}

// ------------------------------------------------------------ Behaviour table tests

#[actix_web::test]
async fn admin_resumes_a_suspended_user() {
    let db = TestDb::start().await;
    allow_suspended(&db).await;
    let user = a_user(&db, "SUSPENDED", &[("TELLER", Some(B01))]).await;
    assert_eq!(db.version(&user).await, 1, "initial version");

    let a = resume(&db, ORG_ADMIN, &user).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "resume suspended user: {}",
        a.text
    );
    assert_eq!(a.body["status"], "ACTIVE");
    assert_eq!(a.body["version"], 2);
    assert_eq!(a.body["userId"], user);

    assert_eq!(db.version(&user).await, 2, "version moved");
    assert_eq!(status_of(&db, &user).await, "ACTIVE", "status is ACTIVE");
    db.finish().await;
}

#[actix_web::test]
async fn resume_an_active_user() {
    let db = TestDb::start().await;
    let user = a_user(&db, "ACTIVE", &[("TELLER", Some(B01))]).await;

    let a = resume(&db, ORG_ADMIN, &user).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "statusNotAllowed",
        "resume active user",
    );
    assert_eq!(db.version(&user).await, 1, "version unchanged");
    assert_eq!(status_of(&db, &user).await, "ACTIVE", "status unchanged");
    db.finish().await;
}

#[actix_web::test]
async fn resume_a_disabled_user() {
    let db = TestDb::start().await;
    let user = a_user(&db, "DISABLED", &[("TELLER", Some(B01))]).await;

    let a = resume(&db, ORG_ADMIN, &user).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "statusNotAllowed",
        "resume disabled user",
    );
    assert_eq!(db.version(&user).await, 1, "version unchanged");
    assert_eq!(status_of(&db, &user).await, "DISABLED", "status unchanged");
    db.finish().await;
}

#[actix_web::test]
async fn a_user_of_another_organisation() {
    let db = TestDb::start().await;
    allow_suspended(&db).await;

    let other_org = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, code, legal_name, timezone, created_at) VALUES ($1, 'OTHER', 'Other Org', 'Asia/Bangkok', now())")
        .bind(other_org)
        .execute(&db.pool)
        .await
        .expect("insert other organisation");
    let other_user: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1, 'Other Org User', 'SUSPENDED', 'test') RETURNING id::text",
    )
    .bind(other_org)
    .fetch_one(&db.pool)
    .await
    .expect("insert user of another organisation");

    let a_resume = resume(&db, ORG_ADMIN, &other_user).await;
    let a_disable = disable(&db, ORG_ADMIN, &other_user).await;

    assert_eq!(
        a_resume.status, a_disable.status,
        "resume status matches user_disable status"
    );
    assert_eq!(
        code(&a_resume),
        code(&a_disable),
        "resume error code matches user_disable code"
    );
    refused(
        &a_resume,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "resume other org user",
    );
    db.finish().await;
}

#[actix_web::test]
async fn no_users_manage_a_teller_is_refused() {
    let db = TestDb::start().await;
    allow_suspended(&db).await;
    let user = a_user(&db, "SUSPENDED", &[("TELLER", Some(B01))]).await;

    let a = resume(&db, "TELLER@XC-B01", &user).await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "TELLER is refused: {}",
        a.text
    );
    assert_eq!(
        code(&a),
        "roleNotAllowedHere",
        "policy refusal code: {}",
        a.text
    );
    assert_eq!(db.version(&user).await, 1, "version unchanged");
    assert_eq!(status_of(&db, &user).await, "SUSPENDED", "status unchanged");
    db.finish().await;
}

#[actix_web::test]
async fn unknown_user_id() {
    let db = TestDb::start().await;
    let unknown_id = Uuid::new_v4().to_string();

    let a_resume = resume(&db, ORG_ADMIN, &unknown_id).await;
    let a_disable = disable(&db, ORG_ADMIN, &unknown_id).await;

    assert_eq!(
        a_resume.status, a_disable.status,
        "resume status matches user_disable status"
    );
    assert_eq!(
        code(&a_resume),
        code(&a_disable),
        "resume error code matches user_disable code"
    );
    refused(
        &a_resume,
        StatusCode::NOT_FOUND,
        "notFound",
        "resume unknown user",
    );
    db.finish().await;
}

#[actix_web::test]
async fn user_enable_on_suspended_user_is_refused() {
    let db = TestDb::start().await;
    allow_suspended(&db).await;
    let user = a_user(&db, "SUSPENDED", &[("TELLER", Some(B01))]).await;

    let a = enable(&db, OWNER, &user).await;
    refused(
        &a,
        StatusCode::CONFLICT,
        "statusNotAllowed",
        "enable suspended user",
    );
    assert_eq!(db.version(&user).await, 1, "version unchanged");
    assert_eq!(status_of(&db, &user).await, "SUSPENDED", "status unchanged");
    db.finish().await;
}
