//! Up-front test for the ticket "a branch-scoped ADMIN can disable a user whose roles reach
//! beyond the admin's branch (an organisation-wide EXECUTIVE, or another branch's staff)"
//! (written by claude/dell-claude-3 for the BE lane controller claude/1656900d, before the fix).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `UserDisableCommand::run` (`api/src/presentation/http/v3/commands/user_disable.rs`) ignores the
//! caller (`_who`) and checks only that the target user is in the branch's organisation
//! (`user_in_organisation`). #2390 / #2393 settle the same gap for grant and revoke with
//! `creator_may_grant`; disabling ends every role the target holds, so the caller must be able
//! to grant each of them.
//!
//! Harness copied from `v3_org_admin_test.rs` (Blueprint callers through `x-test-as`, the whole
//! route table). Needs `TEST_DATABASE_URL`; nothing skips.

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
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ADMIN: &str = "ADMIN@*";
const REFUSED: [&str; 4] = [
    "EXECUTIVE@*",
    "BRANCH_MANAGER@XC-B01",
    "TELLER@XC-B01",
    "FINANCE@XC-B01",
];

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
        let schema = unique("udcreatorscope");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
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

    async fn branch_version(&self, branch: &str) -> i64 {
        self.count(&format!(
            "SELECT COALESCE((SELECT version FROM branch_versions WHERE branch_id = '{branch}'), 0)"
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

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
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

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    call(db, who, Method::GET, uri, None, Key::Fresh).await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn profile(code: &str) -> Value {
    json!({
        "branch": B01,
        "code": code,
        "address": "1 Silom Rd",
        "timezone": "Asia/Bangkok",
        "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
        "bookingEnabled": true
    })
}

/// An ACTIVE TELLER of XC-B01, seeded as #1157's `userCreate` leaves one; answers the user id.
async fn a_teller(db: &TestDb) -> String {
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_person(
        &mut conn,
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        "Somchai",
        &[RoleRow {
            role_code: "TELLER",
            branch_id: Some(B01),
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        }],
    )
    .await
    .expect("a user of XC-B01's organisation")
}

// ------------------------------------------------------------ tests

const BRANCH_ADMIN: &str = "ADMIN@XC-B01";

/// A user holding `role` at `branch` (`None` = organisation-wide); answers the user id.
async fn a_user_with(db: &TestDb, name: &str, role: &str, branch: Option<&str>) -> String {
    let mut conn = db.pool.acquire().await.expect("acquire connection");
    seed_person(
        &mut conn,
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        name,
        &[RoleRow {
            role_code: role,
            branch_id: branch,
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to: None,
        }],
    )
    .await
    .expect("a user of XC-B01's organisation")
}

async fn status_of(db: &TestDb, user: &str) -> String {
    db.text(&format!(
        "SELECT status FROM users WHERE id = '{user}'::uuid"
    ))
    .await
    .expect("the user row")
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

/// R1: an ADMIN scoped to XC-B01 cannot disable an organisation-wide EXECUTIVE.
#[actix_web::test]
async fn a_branch_admin_cannot_disable_an_org_wide_executive() {
    let db = TestDb::start().await;
    let exec = a_user_with(&db, "Exec", "EXECUTIVE", None).await;
    let a = disable(&db, BRANCH_ADMIN, &exec).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(
        status_of(&db, &exec).await,
        "ACTIVE",
        "the executive stays active"
    );
    db.finish().await;
}

/// R2: an ADMIN scoped to XC-B01 cannot disable a TELLER of XC-B09.
#[actix_web::test]
async fn a_branch_admin_cannot_disable_another_branchs_staff() {
    let db = TestDb::start().await;
    let teller = a_user_with(&db, "B09 teller", "TELLER", Some(B09)).await;
    let a = disable(&db, BRANCH_ADMIN, &teller).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert_eq!(status_of(&db, &teller).await, "ACTIVE");
    db.finish().await;
}

/// Kept: the same ADMIN disables a TELLER of its own branch.
#[actix_web::test]
async fn a_branch_admin_still_disables_its_own_branchs_teller() {
    let db = TestDb::start().await;
    let teller = a_user_with(&db, "B01 teller", "TELLER", Some(B01)).await;
    let a = disable(&db, BRANCH_ADMIN, &teller).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(status_of(&db, &teller).await, "DISABLED");
    db.finish().await;
}

/// Changed by RBAC C1a (owner decision Q4 A, PR 3083 §2 and §3.1 rule 3): an organisation-wide
/// ADMIN no longer disables an EXECUTIVE; only EXECUTIVE + ADMIN (the owner, B3) does. The owner's
/// case is in `rbac_c1a_user_commands_test.rs`.
#[actix_web::test]
async fn an_org_wide_admin_alone_cannot_disable_an_executive() {
    let db = TestDb::start().await;
    let exec = a_user_with(&db, "Exec", "EXECUTIVE", None).await;
    let a = disable(&db, ADMIN, &exec).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "roleNotAllowed", "{}", a.text);
    assert_eq!(status_of(&db, &exec).await, "ACTIVE");
    db.finish().await;
}
