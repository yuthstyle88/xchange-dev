//! Up-front test for #2692 "the v3 view wrapper lets an organisation-wide role read another
//! organisation's branch" (written by claude/dell-claude-5 for the BE lane controller
//! claude/1656900d, before the fix). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `handle_branch_view` and `handle_paged_view` (`api/src/presentation/http/v3/view.rs`) authorise
//! the named branch with a policy target built from the caller's own organisation, then check only
//! that the branch exists (`require_branch`). Nothing compares the branch's organisation with the
//! caller's. The read-side twin of #2571 (commands). Probes: `staff_me` (a branch view) and
//! `admin_users` (a paged view), both open to an organisation-wide ADMIN.
//!
//! Harness copied from `command_org_scope_test.rs` (#2571's up-front test). The callers'
//! organisation is the default one (`XC`, id `…0001`). Needs `TEST_DATABASE_URL`; nothing skips.

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
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const FINANCE: &str = "FINANCE@XC-B01";
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
        let schema = unique("vieworgscope");
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
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('00000000-0000-0000-0000-0000000000b2', 'ORG2', 'Asia/Bangkok', now())",
        )
        .execute(&pool)
        .await
        .expect("insert the second organisation");
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) \
             VALUES ('OB1', 'Other org branch', '00000000-0000-0000-0000-0000000000b2')",
        )
        .execute(&pool)
        .await
        .expect("insert the other organisation's branch");
        Self { pool, schema }
    }

    /// The `cash_limits` row of `branch` as `(organization_id, limit text, updated_by, updated_at)`.
    async fn row(&self, branch: &str) -> Option<(String, Option<String>, String, DateTime<Utc>)> {
        sqlx::query_as(
            "SELECT organization_id::text, count_difference_limit_thb::text, updated_by, updated_at \
             FROM cash_limits WHERE branch_id = $1",
        )
        .bind(branch)
        .fetch_optional(&self.pool)
        .await
        .expect("read cash_limits")
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM cash_limits")
            .fetch_one(&self.pool)
            .await
            .expect("count cash_limits")
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
        "XC",
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

/// The legacy actor the seam still carries beside the user (`view.rs:359-371`); its role is not
/// what decides here.
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

enum Key<'a> {
    Fresh,
    This(&'a str),
    Missing,
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
            Key::Missing => {}
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn view(db: &TestDb, who: &str, name: &str, branch: &str) -> Answer {
    call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/{name}?branch={branch}"),
        None,
        Key::Fresh,
    )
    .await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// A user of the other organisation, so a leaked `admin_users` page would show it.
async fn other_org_user(db: &TestDb) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ('00000000-0000-0000-0000-0000000000b2', 'Other org person', 'ACTIVE', 'test') \
         RETURNING id::text",
    )
    .fetch_one(&db.pool)
    .await
    .expect("a user of the other organisation")
}

// ------------------------------------------------------------ tests

const OTHER_BRANCH: &str = "OB1";

/// A branch view: an organisation-wide ADMIN of `XC` names a branch of another organisation and
/// is refused 403 `outOfScope`; the other organisation's branch name is not in the answer.
#[actix_web::test]
async fn a_branch_view_naming_another_organisations_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    let a = view(&db, ADMIN, "staff_me", OTHER_BRANCH).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert!(
        !a.text.contains("Other org branch"),
        "the other organisation's branch name leaks: {}",
        a.text
    );
    db.finish().await;
}

/// A paged view: the same ADMIN names the other organisation's branch on `admin_users` and is
/// refused 403 `outOfScope`; no user of that organisation is listed.
#[actix_web::test]
async fn a_paged_view_naming_another_organisations_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    let other = other_org_user(&db).await;
    let a = view(&db, ADMIN, "admin_users", OTHER_BRANCH).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert!(
        !a.text.contains(&other),
        "a user of the other organisation is listed: {}",
        a.text
    );
    db.finish().await;
}

/// Kept: the same ADMIN still reads its own organisation's branch on both views, and its own
/// organisation's user list does not include the other organisation's user.
#[actix_web::test]
async fn an_org_wide_admin_still_reads_its_own_organisations_branch() {
    let db = TestDb::start().await;
    let other = other_org_user(&db).await;
    let me = view(&db, ADMIN, "staff_me", B09).await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.text);
    let users = view(&db, ADMIN, "admin_users", B09).await;
    assert_eq!(users.status, StatusCode::OK, "{}", users.text);
    assert!(!users.text.contains(&other), "{}", users.text);
    db.finish().await;
}

/// Kept: a branch-scoped caller is unchanged — FINANCE@XC-B01 reads `staff_me` at its branch.
#[actix_web::test]
async fn a_branch_scoped_caller_still_reads_its_own_branch() {
    let db = TestDb::start().await;
    let a = view(&db, FINANCE, "staff_me", B01).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    db.finish().await;
}
