//! RBAC f16 (F4a, #3152): `GET /api/v3/views/admin_user_audit` (`adminUserAudit`).
//!
//! Behaviour table rows:
//! 1. three audit rows in the caller's organisation -> three rows, newest first
//! 2. before = the 2nd row's id -> only older rows
//! 3. a row of another organisation -> not shown
//! 4. an EXECUTIVE or ADMIN (AuditLogsRead) -> 200
//! 5. a caller without the permission -> 403

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
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

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
        let schema = unique("rbac_f4a");
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

struct SeedData {
    id1: String,
    id2: String,
    id3: String,
    foreign_id: String,
}

async fn seed_audit_data(db: &TestDb) -> SeedData {
    let org = xchange::presentation::pages::SEEDED_XC_ORG_ID;

    let actor_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Alice Admin', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(org)
    .fetch_one(&db.pool)
    .await
    .expect("insert actor Alice");

    let subject_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Bob Teller', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(org)
    .fetch_one(&db.pool)
    .await
    .expect("insert subject Bob");

    let id1: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id, before, after, ip) \
         VALUES ($1, $2, 'userCreate', $3, $4::uuid, NULL, $5::text::jsonb, '10.0.0.1'::inet) RETURNING id::text",
    )
    .bind(&actor_id)
    .bind(&subject_id)
    .bind(at(10))
    .bind(org)
    .bind(json!({"status": "ACTIVE"}).to_string())
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 1");

    let id2: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id, before, after, ip) \
         VALUES ('system', $1, 'userProfileSave', $2, $3::uuid, $4::text::jsonb, $5::text::jsonb, '192.168.1.5'::inet) RETURNING id::text",
    )
    .bind(&subject_id)
    .bind(at(20))
    .bind(org)
    .bind(json!({"employee_code": null}).to_string())
    .bind(json!({"employee_code": "E1"}).to_string())
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 2");

    let id3: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ($1, 'system-subj', 'userDisable', $2, $3::uuid) RETURNING id::text",
    )
    .bind(&actor_id)
    .bind(at(30))
    .bind(org)
    .fetch_one(&db.pool)
    .await
    .expect("insert audit 3");

    // Second organisation
    let other_org_id = "00000000-0000-0000-0000-0000000000b2";
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'YY', NULL, 'Asia/Bangkok', now())",
    )
    .bind(other_org_id)
    .execute(&db.pool)
    .await
    .expect("insert second org");

    let foreign_id: String = sqlx::query_scalar(
        "INSERT INTO user_audit (actor_id, subject_id, action, at, organization_id) \
         VALUES ('other-actor', 'other-subj', 'userCreate', $1, $2::uuid) RETURNING id::text",
    )
    .bind(at(40))
    .bind(other_org_id)
    .fetch_one(&db.pool)
    .await
    .expect("insert foreign audit");

    SeedData {
        id1,
        id2,
        id3,
        foreign_id,
    }
}

// ------------------------------------------------------------ behaviour table tests

/// 1. three audit rows in the caller's organisation -> three rows, newest first
#[actix_web::test]
async fn case1_three_audit_rows_in_callers_organisation_returns_newest_first() {
    let db = TestDb::start().await;
    let seed = seed_audit_data(&db).await;

    let ans = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_user_audit?branch={B01}"),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of audit rows");
    assert_eq!(data.len(), 3, "expected 3 audit rows, got: {:?}", data);

    // Newest first: id3 (at 30), then id2 (at 20), then id1 (at 10)
    assert_eq!(data[0]["id"], seed.id3);
    assert_eq!(data[0]["action"], "userDisable");
    assert_eq!(data[0]["actorName"], "Alice Admin");
    assert_eq!(data[0]["subjectName"], "system-subj");

    assert_eq!(data[1]["id"], seed.id2);
    assert_eq!(data[1]["action"], "userProfileSave");
    assert_eq!(data[1]["actorName"], "system");
    assert_eq!(data[1]["subjectName"], "Bob Teller");
    assert_eq!(data[1]["ip"], "192.168.1.5");
    assert_eq!(data[1]["before"]["employee_code"], Value::Null);
    assert_eq!(data[1]["after"]["employee_code"], "E1");

    assert_eq!(data[2]["id"], seed.id1);
    assert_eq!(data[2]["action"], "userCreate");
    assert_eq!(data[2]["actorName"], "Alice Admin");
    assert_eq!(data[2]["subjectName"], "Bob Teller");
    assert_eq!(data[2]["ip"], "10.0.0.1");
    assert_eq!(data[2]["before"], Value::Null);
    assert_eq!(data[2]["after"]["status"], "ACTIVE");

    db.finish().await;
}

/// 2. before = the 2nd row's id -> only older rows
#[actix_web::test]
async fn case2_before_second_row_id_returns_only_older_rows() {
    let db = TestDb::start().await;
    let seed = seed_audit_data(&db).await;

    let ans = get(
        &db,
        ADMIN,
        &format!(
            "/api/v3/views/admin_user_audit?branch={B01}&before={}",
            seed.id2
        ),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of audit rows");
    assert_eq!(data.len(), 1, "expected only 1 older row, got: {:?}", data);
    assert_eq!(data[0]["id"], seed.id1);

    db.finish().await;
}

/// 3. a row of another organisation -> not shown
#[actix_web::test]
async fn case3_row_of_another_organisation_not_shown() {
    let db = TestDb::start().await;
    let seed = seed_audit_data(&db).await;

    let ans = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_user_audit?branch={B01}"),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "response: {}", ans.text);

    let data = ans.body["data"].as_array().expect("array of audit rows");
    for row in data {
        assert_ne!(
            row["id"], seed.foreign_id,
            "foreign audit row must not be shown"
        );
    }

    db.finish().await;
}

/// 4. an EXECUTIVE or ADMIN (AuditLogsRead) -> 200
#[actix_web::test]
async fn case4_executive_or_admin_audit_logs_read_answers_200() {
    let db = TestDb::start().await;
    let _seed = seed_audit_data(&db).await;

    let ans_admin = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_user_audit?branch={B01}"),
    )
    .await;
    assert_eq!(
        ans_admin.status,
        StatusCode::OK,
        "admin must get 200: {}",
        ans_admin.text
    );

    let ans_exec = get(
        &db,
        EXECUTIVE,
        &format!("/api/v3/views/admin_user_audit?branch={B01}"),
    )
    .await;
    assert_eq!(
        ans_exec.status,
        StatusCode::OK,
        "executive must get 200: {}",
        ans_exec.text
    );

    db.finish().await;
}

/// 5. a caller without the permission -> 403 / 400 policy refusal
#[actix_web::test]
async fn case5_caller_without_permission_answers_403() {
    let db = TestDb::start().await;
    let _seed = seed_audit_data(&db).await;

    let ans = get(
        &db,
        TELLER,
        &format!("/api/v3/views/admin_user_audit?branch={B01}"),
    )
    .await;
    assert!(
        ans.status == StatusCode::FORBIDDEN || ans.status == StatusCode::BAD_REQUEST,
        "caller without permission must be refused: {}",
        ans.text
    );
    assert_eq!(ans.body["error"]["code"], "roleNotAllowedHere");

    db.finish().await;
}
