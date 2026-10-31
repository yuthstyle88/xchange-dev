//! #5684 (design `docs/design/org-wide-actor-branch.md` §16, rows A-1..A-7): `GET /api/v3/views/admin_branches`
//! lists only the branches the caller's rows for the view's permission reach (`BranchScope::of(who, CompatRead)`),
//! not every branch any live row reaches. Ruling N-8 on #5558 (issuecomment-6515987193): "Filtering it to the
//! caller's own branches is a follow-up, as #5557 did". Written before the implementation by the ticket author
//! (claude/86094638). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! A caller is `ROW+ROW..`, each row `ROLE@BRANCH` or `ROLE@*` (organisation-wide), with a trailing `!` for a row
//! that ended yesterday. The branches: `MAIN` (from the migration), `XC-B01`, `XC-B02`, `XC-B03`, all in the seeded
//! organisation. Each call names `branch=XC-B01`, where every caller here holds ADMIN.

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

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const URI: &str = "/api/v3/views/admin_branches?branch=XC-B01";
const ALL: [&str; 4] = ["MAIN", "XC-B01", "XC-B02", "XC-B03"];

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
        let schema = xc_testdb::unique_schema_name("adminbranchesscope");
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
        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02'), ('XC-B03', 'B03')",
        )
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

fn role_of(code: &str) -> Role {
    match code {
        "ADMIN" => Role::Admin,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        other => panic!("unknown role {other}"),
    }
}

/// `ROLE@BRANCH` or `ROLE@*`, joined by `+`; a trailing `!` ends the row yesterday.
fn user_of(spec: &str) -> Option<User> {
    if spec.is_empty() {
        return None;
    }
    let epoch = DateTime::<Utc>::UNIX_EPOCH;
    let yesterday = Utc::now() - chrono::Duration::days(1);
    let rows: Vec<ScopedRole> = spec
        .split('+')
        .map(|row| {
            let (row, ended) = match row.strip_suffix('!') {
                Some(r) => (r, true),
                None => (row, false),
            };
            let (role, at) = row.split_once('@').expect("ROLE@BRANCH");
            let branch = (at != "*").then(|| BranchId::new(at));
            ScopedRole::new(role_of(role), branch, epoch, ended.then_some(yesterday))
        })
        .collect();
    Some(
        User::new(
            &format!("user-{}", spec.replace(['@', '*', '+', '!'], "-")),
            xchange::presentation::pages::SEEDED_XC_ORG_ID,
            rows,
            None,
        )
        .with_branch(Some(BranchId::new("XC-B01"))),
    )
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move { test_callers::callers(None, user_of(&spec)) })
    }
}

// ------------------------------------------------------------ requests

/// The branch ids `who` gets from the view, as a set (the order is `v3_admin_branches_view_test`'s concern).
async fn listed(db: &TestDb, who: &str) -> BTreeSet<String> {
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
        .uri(URI)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{who}: {text}");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    body["data"]["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("{who}: no data.branches in {text}"))
        .iter()
        .map(|b| b["id"].as_str().expect("id").to_owned())
        .collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

async fn check(who: &str, expected: &[&str]) {
    let db = TestDb::start().await;
    let got = listed(&db, who).await;
    db.finish().await;
    assert_eq!(got, set(expected), "{who}");
}

// ------------------------------------------------------------ rows

/// A-1: a branch ADMIN alone sees its branch (green today).
#[actix_web::test]
async fn a1_branch_admin_sees_its_branch() {
    check("ADMIN@XC-B01", &["XC-B01"]).await;
}

/// A-2: an ADMIN at two branches sees both (green today).
#[actix_web::test]
async fn a2_two_branch_admin_sees_both() {
    check("ADMIN@XC-B01+ADMIN@XC-B02", &["XC-B01", "XC-B02"]).await;
}

/// A-3: an organisation-wide ADMIN sees every branch of the organisation (green today).
#[actix_web::test]
async fn a3_org_admin_sees_every_branch() {
    check("ADMIN@*", &ALL).await;
}

/// A-4: an organisation-wide row whose group lacks `CompatRead` (MARKETING) does not widen the list (red today:
/// `SessionBranch::switchable` counts every live row, so it answers all four).
#[actix_web::test]
async fn a4_org_wide_row_without_the_permission_does_not_widen() {
    check("ADMIN@XC-B01+MARKETING@*", &["XC-B01"]).await;
}

/// A-5: an organisation-wide row that holds `CompatRead` (TELLER) reaches every branch (green today).
#[actix_web::test]
async fn a5_org_wide_row_with_the_permission_reaches_every_branch() {
    check("ADMIN@XC-B01+TELLER@*", &ALL).await;
}

/// A-6: an ended organisation-wide ADMIN row does not widen the list (green today).
#[actix_web::test]
async fn a6_ended_org_wide_row_does_not_widen() {
    check("ADMIN@XC-B01+ADMIN@*!", &["XC-B01"]).await;
}

/// A-7: a branch row whose group lacks `CompatRead` (MARKETING @ XC-B02) does not add its branch (red today).
#[actix_web::test]
async fn a7_branch_row_without_the_permission_does_not_add_its_branch() {
    check("ADMIN@XC-B01+MARKETING@XC-B02", &["XC-B01"]).await;
}
