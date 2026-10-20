//! Up-front test for the ticket "the v3 command wrapper lets an organisation-wide role write to
//! another organisation's branch" (written by claude/dell-claude-3 for the BE lane controller
//! claude/1656900d, before the fix). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `handle_command` (`api/src/presentation/http/v3/command.rs`) authorises the named branch with a
//! policy target built from the caller's own organisation, checks only that the branch exists
//! (`require_branch`), then reads the organisation from the branch itself and runs the command
//! under it. Nothing compares that organisation with the caller's. `PUT /api/cash-limits`
//! (`cashLimitsSave`, a plain `Locks::One` command with no check of its own) is the probe.
//!
//! Harness copied from `v3_cash_limits_test.rs` (Blueprint callers through `x-test-as`, the whole
//! route table). The callers' organisation is the default one (`XC`, id `…0001`), as in that file.
//! Needs `TEST_DATABASE_URL`; nothing skips.

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
        let schema = unique("cmdorgscope");
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
            let _s = legacy_of(&spec);
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

async fn save_with(db: &TestDb, who: &str, branch: &str, limit: Value, key: Key<'_>) -> Answer {
    call(
        db,
        who,
        Method::PUT,
        "/api/cash-limits",
        Some(json!({ "branch": branch, "countDifferenceLimitThb": limit })),
        key,
    )
    .await
}

async fn save(db: &TestDb, who: &str, branch: &str, limit: Value) -> Answer {
    save_with(db, who, branch, limit, Key::Fresh).await
}

async fn read(db: &TestDb, who: &str, branch: &str) -> Answer {
    call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/cash_limits?branch={branch}"),
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

// ------------------------------------------------------------ tests

const OTHER_BRANCH: &str = "OB1";

/// An organisation-wide ADMIN of `XC` names a branch of another organisation: refused 403
/// `outOfScope`, and nothing is written for that branch.
#[actix_web::test]
async fn an_org_wide_admin_cannot_write_another_organisations_branch() {
    let db = TestDb::start().await;
    let a = save(&db, ADMIN, OTHER_BRANCH, json!("500")).await;
    assert_eq!(a.status, StatusCode::FORBIDDEN, "{}", a.text);
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    assert!(
        db.row(OTHER_BRANCH).await.is_none(),
        "no cash_limits row is written for the other organisation's branch"
    );
    db.finish().await;
}

/// Kept: the same ADMIN still writes a branch of its own organisation.
#[actix_web::test]
async fn an_org_wide_admin_still_writes_its_own_organisations_branch() {
    let db = TestDb::start().await;
    let a = save(&db, ADMIN, B09, json!("7")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let (org, limit, _, _) = db.row(B09).await.expect("one row");
    assert_eq!(org, ORG);
    assert_eq!(limit.as_deref(), Some("7.00"));
    db.finish().await;
}
