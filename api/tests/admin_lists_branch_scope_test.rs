//! #5557 T2 (design `docs/design/org-wide-actor-branch.md` §11, rows V-1..V-9): a branch-scoped ADMIN sees in
//! `admin_users`, `admin_employee_register`, `admin_user_audit` and `admin_login_history` only the users its rows
//! cover; an organisation-wide ADMIN still sees everyone. Owner's B-2 answer (#5526, 2026-10-30):
//! "กรองให้เห็นเฉพาะสาขาตัวเอง". Written before the implementation by the ticket author (claude/2ca5874b). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! The users (§11.4), named so that `b2` sorts between `b1` and `b1b`: `b1` (U1) TELLER @ B01; `b1b` TELLER @ B01 and BRANCH_MANAGER @ B01; `b2` TELLER @ B02; `both`
//! TELLER @ B01 and @ B02; `org` FINANCE organisation-wide; `none` no row; `ended` a TELLER @ B01 row that ended.

mod common;

use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

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

const B01: &str = "XC-B01";
const BRANCH_ADMIN: &str = "ADMIN@XC-B01";
const TWO_BRANCH_ADMIN: &str = "ADMIN@XC-B01,XC-B02";
const ORG_ADMIN: &str = "ADMIN@*";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";

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
        let schema = unique("adminlistsscope");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02'), ('XC-B03', 'B03')")
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
    format!("user-{}", spec.replace(['@', '*', ','], "-"))
}

/// `ROLE@BRANCH[,BRANCH..]` or `ROLE@*`: one live row per branch (or one organisation-wide row).
fn user_of(spec: &str) -> Option<User> {
    let (role, at) = spec.split_once('@')?;
    let role = match role {
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        other => panic!("unknown role {other}"),
    };
    let rows: Vec<Option<BranchId>> = if at == "*" {
        vec![None]
    } else {
        at.split(',').map(|b| Some(BranchId::new(b))).collect()
    };
    let session_branch = rows[0].clone().unwrap_or_else(|| BranchId::new("MAIN"));
    Some(
        User::new(
            &user_id(spec),
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            rows.into_iter()
                .map(|branch| ScopedRole::new(role, branch, DateTime::<Utc>::UNIX_EPOCH, None))
                .collect(),
            None,
        )
        .with_branch(Some(session_branch)),
    )
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

// ------------------------------------------------------------ the users

struct Users {
    b1: String,
    b1b: String,
    b2: String,
    both: String,
    org: String,
    none: String,
    ended: String,
}

async fn insert_user(db: &TestDb, name: &str, code: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, employee_code, status, created_by) \
         VALUES ($1::uuid, $2, $3, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(xchange::presentation::pages::SEEDED_XC_ORG_ID)
    .bind(name)
    .bind(code)
    .fetch_one(&db.pool)
    .await
    .expect("insert user")
}

async fn insert_role(db: &TestDb, user: &str, role: &str, branch: Option<&str>, ended: bool) {
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from, valid_to) \
         VALUES ($1::uuid, $2, $3::uuid, $4, now() - interval '2 days', \
                 CASE WHEN $5 THEN now() - interval '1 day' END)",
    )
    .bind(user)
    .bind(role)
    .bind(xchange::presentation::pages::SEEDED_XC_ORG_ID)
    .bind(branch)
    .bind(ended)
    .execute(&db.pool)
    .await
    .expect("insert role");
}

async fn seed(db: &TestDb) -> Users {
    let b1 = insert_user(db, "U1", "E01").await;
    insert_role(db, &b1, "TELLER", Some(B01), false).await;
    let b1b = insert_user(db, "U3", "E02").await;
    insert_role(db, &b1b, "TELLER", Some(B01), false).await;
    insert_role(db, &b1b, "BRANCH_MANAGER", Some(B01), false).await;
    let b2 = insert_user(db, "U2", "E03").await;
    insert_role(db, &b2, "TELLER", Some("XC-B02"), false).await;
    let both = insert_user(db, "U4", "E04").await;
    insert_role(db, &both, "TELLER", Some(B01), false).await;
    insert_role(db, &both, "TELLER", Some("XC-B02"), false).await;
    let org = insert_user(db, "U5", "E05").await;
    insert_role(db, &org, "FINANCE", None, false).await;
    let none = insert_user(db, "U6", "E06").await;
    let ended = insert_user(db, "U7", "E07").await;
    insert_role(db, &ended, "TELLER", Some(B01), true).await;
    Users {
        b1,
        b1b,
        b2,
        both,
        org,
        none,
        ended,
    }
}

/// One `user_audit` row and one `login_history` row per user, plus an audit row whose subject is no user and a
/// login of an unknown identifier (no `user_id`).
async fn seed_history(db: &TestDb, users: &Users) {
    let org = xchange::presentation::pages::SEEDED_XC_ORG_ID;
    for u in [
        &users.b1,
        &users.b1b,
        &users.b2,
        &users.both,
        &users.org,
        &users.none,
        &users.ended,
    ] {
        sqlx::query(
            "INSERT INTO user_audit (actor_id, subject_id, action, organization_id) \
             VALUES ('test', $1, 'profileSave', $2::uuid)",
        )
        .bind(u)
        .bind(org)
        .execute(&db.pool)
        .await
        .expect("insert audit");
        sqlx::query(
            "INSERT INTO login_history (organization_id, user_id, identifier_hash, result) \
             VALUES ($1::uuid, $2::uuid, 'h', 'OK')",
        )
        .bind(org)
        .bind(u)
        .execute(&db.pool)
        .await
        .expect("insert login");
    }
    sqlx::query(
        "INSERT INTO user_audit (actor_id, subject_id, action, organization_id) \
         VALUES ('test', 'not-a-user', 'profileSave', $1::uuid)",
    )
    .bind(org)
    .execute(&db.pool)
    .await
    .expect("insert audit of no user");
    sqlx::query(
        "INSERT INTO login_history (organization_id, user_id, identifier_hash, result) \
         VALUES ($1::uuid, NULL, 'h', 'UNKNOWN_USER')",
    )
    .bind(org)
    .execute(&db.pool)
    .await
    .expect("insert unknown login");
}

/// The values of `field` over every row of `view` (one page of 100), as `who`, naming B01.
async fn listed(db: &TestDb, who: &str, view: &str, field: &str) -> Vec<Value> {
    let r = get(
        db,
        who,
        &format!("/api/v3/views/{view}?branch={B01}&limit=100"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{who} on {view}: {}", r.text);
    r.body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("{view} rows: {}", r.text))
        .iter()
        .map(|row| row[field].clone())
        .collect()
}

fn set(values: &[Value]) -> BTreeSet<String> {
    values
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .collect()
}

fn ids(users: &[&String]) -> BTreeSet<String> {
    users.iter().map(|u| (*u).clone()).collect()
}

// ------------------------------------------------------------ behaviour table

#[actix_web::test]
async fn v1_a_branch_admin_lists_only_the_users_whose_live_rows_are_all_on_its_branch() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    let got = listed(&db, BRANCH_ADMIN, "admin_users", "userId").await;
    assert_eq!(
        set(&got),
        ids(&[&u.b1, &u.b1b]),
        "B01 admin: b1 and b1b only"
    );
    db.finish().await;
}

#[actix_web::test]
async fn v2_an_admin_of_two_branches_lists_the_users_of_either_and_of_both() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    let got = listed(&db, TWO_BRANCH_ADMIN, "admin_users", "userId").await;
    assert_eq!(set(&got), ids(&[&u.b1, &u.b1b, &u.b2, &u.both]));
    db.finish().await;
}

#[actix_web::test]
async fn v3_an_organisation_wide_admin_still_lists_every_user() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    let got = listed(&db, ORG_ADMIN, "admin_users", "userId").await;
    assert_eq!(
        set(&got),
        ids(&[&u.b1, &u.b1b, &u.b2, &u.both, &u.org, &u.none, &u.ended])
    );
    db.finish().await;
}

#[actix_web::test]
async fn v4_a_branch_admin_pages_through_its_users_without_a_gap_or_a_repeat() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    let first = get(
        &db,
        BRANCH_ADMIN,
        &format!("/api/v3/views/admin_users?branch={B01}&limit=1"),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let first = first.body["data"].as_array().expect("rows").clone();
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0]["userId"],
        u.b1.as_str(),
        "ordered by name: U1 (b1) first"
    );
    let next = get(
        &db,
        BRANCH_ADMIN,
        &format!(
            "/api/v3/views/admin_users?branch={B01}&limit=1&before={}",
            u.b1
        ),
    )
    .await;
    assert_eq!(next.status, StatusCode::OK, "{}", next.text);
    let next = next.body["data"].as_array().expect("rows").clone();
    assert_eq!(next.len(), 1);
    assert_eq!(
        next[0]["userId"],
        u.b1b.as_str(),
        "the next covered user U3 (b1b), not U2 (b2), which sorts between them"
    );
    db.finish().await;
}

#[actix_web::test]
async fn v5_the_employee_register_follows_the_same_users() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    let got = listed(&db, BRANCH_ADMIN, "admin_employee_register", "userId").await;
    assert_eq!(set(&got), ids(&[&u.b1, &u.b1b]));
    let got = listed(&db, ORG_ADMIN, "admin_employee_register", "userId").await;
    assert_eq!(
        set(&got),
        ids(&[&u.b1, &u.b1b, &u.b2, &u.both, &u.org, &u.none, &u.ended])
    );
    db.finish().await;
}

#[actix_web::test]
async fn v6_a_branch_admin_reads_the_audit_rows_of_its_users_only() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    seed_history(&db, &u).await;
    let got = listed(&db, BRANCH_ADMIN, "admin_user_audit", "subjectId").await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(set(&got), ids(&[&u.b1, &u.b1b]));
    db.finish().await;
}

#[actix_web::test]
async fn v7_a_branch_admin_reads_the_logins_of_its_users_only() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    seed_history(&db, &u).await;
    let got = listed(&db, BRANCH_ADMIN, "admin_login_history", "userId").await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(set(&got), ids(&[&u.b1, &u.b1b]));
    db.finish().await;
}

#[actix_web::test]
async fn v8_an_organisation_wide_admin_still_reads_every_audit_row_and_login() {
    let db = TestDb::start().await;
    let u = seed(&db).await;
    seed_history(&db, &u).await;
    let audit = listed(&db, ORG_ADMIN, "admin_user_audit", "subjectId").await;
    assert_eq!(audit.len(), 8, "7 users and the row of no user: {audit:?}");
    let logins = listed(&db, ORG_ADMIN, "admin_login_history", "userId").await;
    assert_eq!(
        logins.len(),
        8,
        "7 users and the unknown identifier: {logins:?}"
    );
    db.finish().await;
}
