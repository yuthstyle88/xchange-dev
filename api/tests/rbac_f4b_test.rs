//! RBAC f17 (F4b, #3153): `GET /api/v3/views/admin_login_history` (`adminLoginHistory`).
//!
//! Behaviour table rows:
//! 1. three login rows in the caller's organisation -> newest first
//! 2. a row with no user -> userId and userName null
//! 3. before paging -> only older rows
//! 4. a row of another organisation, or with organization_id NULL -> not shown
//! 5. a caller without the permission -> 403 / 400 policy refusal

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
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const B01: &str = "XC-B01";
const ADMIN: &str = "ADMIN@*";
const EXECUTIVE: &str = "EXECUTIVE@*";
const TELLER: &str = "TELLER@XC-B01";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn at(sec: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + sec, 0).expect("valid timestamp")
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
        let schema = unique("rbac_f4b");
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

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

// ------------------------------------------------------------ seed helpers

struct SeedData {
    id1: String,
    id2: String,
    id3: String,
    id_anonymous: String,
    foreign_id: String,
    null_org_id: String,
    user_id: String,
}

async fn seed_login_data(db: &TestDb) -> SeedData {
    let org_id: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'XC-B01'")
            .fetch_one(&db.pool)
            .await
            .expect("XC-B01 organisation");

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1, 'Bob Teller', 'ACTIVE', 'test') RETURNING id",
    )
    .bind(org_id)
    .fetch_one(&db.pool)
    .await
    .expect("insert user");

    // Login 1: at sec 10, user_id, OK, 10.0.0.1
    let id1: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1, $2, 'hash1', '10.0.0.1'::inet, 'OK', $3) RETURNING id::text",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(at(10))
    .fetch_one(&db.pool)
    .await
    .expect("login 1");

    // Login 2: at sec 20, user_id, BAD_PASSWORD, 10.0.0.2
    let id2: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1, $2, 'hash2', '10.0.0.2'::inet, 'BAD_PASSWORD', $3) RETURNING id::text",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(at(20))
    .fetch_one(&db.pool)
    .await
    .expect("login 2");

    // Login 3: at sec 30, user_id, OK, 10.0.0.3
    let id3: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1, $2, 'hash3', '10.0.0.3'::inet, 'OK', $3) RETURNING id::text",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(at(30))
    .fetch_one(&db.pool)
    .await
    .expect("login 3");

    // Anonymous login: at sec 25, user_id NULL, UNKNOWN_USER, 192.168.1.1
    let id_anonymous: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1, NULL, 'hash_anon', '192.168.1.1'::inet, 'UNKNOWN_USER', $2) RETURNING id::text",
    )
    .bind(org_id)
    .bind(at(25))
    .fetch_one(&db.pool)
    .await
    .expect("login anonymous");

    // Foreign org login:
    let other_org_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'OTHER', 'Other Org', 'Asia/Bangkok', now())",
    )
    .bind(other_org_id)
    .execute(&db.pool)
    .await
    .expect("insert other org");

    let foreign_id: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES ($1::uuid, NULL, 'hash_foreign', '203.0.113.1'::inet, 'OK', $2) RETURNING id::text",
    )
    .bind(other_org_id)
    .bind(at(40))
    .fetch_one(&db.pool)
    .await
    .expect("insert foreign login");

    // Null organization_id login:
    let null_org_id: String = sqlx::query_scalar(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, ip, result, at) \
         VALUES (NULL, NULL, 'hash_null', '198.51.100.1'::inet, 'UNAVAILABLE', $1) RETURNING id::text",
    )
    .bind(at(50))
    .fetch_one(&db.pool)
    .await
    .expect("insert null org login");

    SeedData {
        id1,
        id2,
        id3,
        id_anonymous,
        foreign_id,
        null_org_id,
        user_id: user_id.to_string(),
    }
}

// ------------------------------------------------------------ behaviour table tests

/// 1. three login rows in the caller's organisation -> newest first
#[actix_web::test]
async fn three_login_rows_in_callers_organisation_returns_newest_first() {
    let db = TestDb::start().await;
    let seed = seed_login_data(&db).await;

    let ans = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_login_history?branch={B01}"),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of login rows");
    // All 4 rows of caller's org: id3 (at 30), id_anonymous (at 25), id2 (at 20), id1 (at 10)
    assert_eq!(data.len(), 4, "expected 4 login rows, got: {:?}", data);

    // Newest first:
    assert_eq!(data[0]["id"], seed.id3);
    assert_eq!(data[0]["result"], "OK");
    assert_eq!(data[0]["userId"], seed.user_id);
    assert_eq!(data[0]["userName"], "Bob Teller");
    assert_eq!(data[0]["ip"], "10.0.0.3");

    assert_eq!(data[1]["id"], seed.id_anonymous);
    assert_eq!(data[1]["result"], "UNKNOWN_USER");

    assert_eq!(data[2]["id"], seed.id2);
    assert_eq!(data[2]["result"], "BAD_PASSWORD");
    assert_eq!(data[2]["userId"], seed.user_id);
    assert_eq!(data[2]["userName"], "Bob Teller");
    assert_eq!(data[2]["ip"], "10.0.0.2");

    assert_eq!(data[3]["id"], seed.id1);
    assert_eq!(data[3]["result"], "OK");
    assert_eq!(data[3]["userId"], seed.user_id);
    assert_eq!(data[3]["userName"], "Bob Teller");
    assert_eq!(data[3]["ip"], "10.0.0.1");

    db.finish().await;
}

/// 2. a row with no user -> userId and userName null
#[actix_web::test]
async fn row_with_no_user_has_user_id_and_user_name_null() {
    let db = TestDb::start().await;
    let seed = seed_login_data(&db).await;

    let ans = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_login_history?branch={B01}"),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of login rows");
    let anon_row = data
        .iter()
        .find(|r| r["id"] == seed.id_anonymous)
        .expect("anonymous login row");

    assert_eq!(anon_row["userId"], Value::Null);
    assert_eq!(anon_row["userName"], Value::Null);
    assert_eq!(anon_row["ip"], "192.168.1.1");
    assert_eq!(anon_row["result"], "UNKNOWN_USER");

    db.finish().await;
}

/// 3. before paging -> only older rows
#[actix_web::test]
async fn before_paging_returns_only_older_rows() {
    let db = TestDb::start().await;
    let seed = seed_login_data(&db).await;

    // before id2 (at 20): only id1 (at 10) is older
    let ans = get(
        &db,
        ADMIN,
        &format!(
            "/api/v3/views/admin_login_history?branch={B01}&before={}",
            seed.id2
        ),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of login rows");
    assert_eq!(data.len(), 1, "expected 1 older row, got: {:?}", data);
    assert_eq!(data[0]["id"], seed.id1);

    db.finish().await;
}

/// 4. a row of another organisation, or with organization_id NULL -> not shown
#[actix_web::test]
async fn row_of_another_organisation_or_null_org_not_shown() {
    let db = TestDb::start().await;
    let seed = seed_login_data(&db).await;

    let ans = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_login_history?branch={B01}"),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of login rows");
    for row in data {
        assert_ne!(
            row["id"], seed.foreign_id,
            "foreign login row must not be shown"
        );
        assert_ne!(
            row["id"], seed.null_org_id,
            "null org login row must not be shown"
        );
    }

    db.finish().await;
}

/// 5. a caller without the permission -> 403 / 400 policy refusal
#[actix_web::test]
async fn caller_without_permission_answers_403() {
    let db = TestDb::start().await;
    let _seed = seed_login_data(&db).await;

    let ans = get(
        &db,
        TELLER,
        &format!("/api/v3/views/admin_login_history?branch={B01}"),
    )
    .await;
    assert!(
        ans.status == StatusCode::FORBIDDEN || ans.status == StatusCode::BAD_REQUEST,
        "caller without permission must be refused: {}",
        ans.text
    );
    assert_eq!(code(&ans), "roleNotAllowedHere");

    db.finish().await;
}

/// Bonus check: EXECUTIVE with AuditLogsRead also gets 200
#[actix_web::test]
async fn executive_with_audit_logs_read_answers_200() {
    let db = TestDb::start().await;
    let _seed = seed_login_data(&db).await;

    let ans = get(
        &db,
        EXECUTIVE,
        &format!("/api/v3/views/admin_login_history?branch={B01}"),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "executive response: {}",
        ans.text
    );

    db.finish().await;
}
