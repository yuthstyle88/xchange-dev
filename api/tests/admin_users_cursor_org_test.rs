//! Ticket #2698: admin_users view before cursor organisation scoping.
//!
//! Behaviour table from issue #2698 §8:
//! C1: ADMIN@* of default org, branch=XC-B01&before=<a user of a second organisation> -> rows: [] (main: own users sorting after that name)
//! C2: before=<unknown uuid> -> rows: [] (kept)
//! C3: before=<an own-organisation user> -> the next page (kept)

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
        let schema = unique("adminuserscursor");
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

fn legacy_of(spec: &str) -> Sessions {
    match spec.split_once('@') {
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

async fn seed_test_users(db: &TestDb) -> (String, String, String) {
    // Insert two users into SEEDED_XC_ORG_ID
    let user_alpha: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1, 'Alpha', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(Uuid::parse_str(xchange::presentation::pages::SEEDED_XC_ORG_ID).unwrap())
    .fetch_one(&db.pool)
    .await
    .expect("insert user Alpha");

    let user_omega: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1, 'Omega', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(Uuid::parse_str(xchange::presentation::pages::SEEDED_XC_ORG_ID).unwrap())
    .fetch_one(&db.pool)
    .await
    .expect("insert user Omega");

    // Insert a second organisation and a user in that organisation
    let other_org_id = "00000000-0000-0000-0000-0000000000b2";
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'YY', NULL, 'Asia/Bangkok', now())",
    )
    .bind(other_org_id)
    .execute(&db.pool)
    .await
    .expect("insert second org");

    let foreign_user: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Beta', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(other_org_id)
    .fetch_one(&db.pool)
    .await
    .expect("insert foreign user Beta");

    (user_alpha, user_omega, foreign_user)
}

// ------------------------------------------------------------ behaviour table tests

/// C1: ADMIN@* of the default org, branch=XC-B01&before=<a user of a second organisation>
/// Required: rows: [] (main: own users sorting after that name)
#[actix_web::test]
async fn c1_before_cursor_naming_foreign_org_user_answers_empty_rows() {
    let db = TestDb::start().await;
    let (_alpha, _omega, foreign_user) = seed_test_users(&db).await;

    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&before={foreign_user}"),
    )
    .await;

    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        0,
        "a foreign organisation user cursor must answer empty rows, but got: {:?}",
        rows
    );

    db.finish().await;
}

/// C2: before=<unknown uuid>
/// Required: rows: [] (kept)
#[actix_web::test]
async fn c2_before_cursor_naming_unknown_uuid_answers_empty_rows() {
    let db = TestDb::start().await;
    let (_alpha, _omega, _foreign) = seed_test_users(&db).await;

    let unknown_uuid = Uuid::new_v4().to_string();
    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&before={unknown_uuid}"),
    )
    .await;

    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        0,
        "an unknown uuid cursor must answer empty rows, but got: {:?}",
        rows
    );

    db.finish().await;
}

/// C3: before=<an own-organisation user>
/// Required: the next page (kept)
#[actix_web::test]
async fn c3_before_cursor_naming_own_org_user_answers_next_page() {
    let db = TestDb::start().await;
    let (alpha, omega, _foreign) = seed_test_users(&db).await;

    let r = get(
        &db,
        ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&before={alpha}"),
    )
    .await;

    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let rows = r.body["data"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        1,
        "paging after Alpha must return exactly Omega: {:?}",
        rows
    );
    assert_eq!(rows[0]["userId"], omega);
    assert_eq!(rows[0]["name"], "Omega");

    db.finish().await;
}
