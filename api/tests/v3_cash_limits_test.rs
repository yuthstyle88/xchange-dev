//! LG-3b (#926, ledger design §3.3, §11 Q-2): `PUT /api/cash-limits` (`cashLimitsSave`) and the
//! `cash_limits` view. Only FINANCE and ADMIN may set or read a branch's count-difference limit,
//! never BRANCH_MANAGER, whose own counts the limit decides on.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The callers are Blueprint `access::Actor`s with exactly the roles under test, handed to the
//! seam through `SessionReader::callers` (OR-4b: views and commands authorise the `user`). A policy
//! refusal is pinned by its code, not its HTTP status, which `PolicyError::status()` decides. Needs
//! `TEST_DATABASE_URL`; nothing skips.

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
use xc_auth::domain::access::{Actor as User, Permission, Role, RoleGrants, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const FINANCE: &str = "FINANCE@XC-B01";
// §11.2: CashLimitsManage (FINANCE FULL) is held by EXECUTIVE and FINANCE, not ADMIN.
const EXECUTIVE: &str = "EXECUTIVE@*";

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
        let schema = unique("v3cashlimits");
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
        "RATE_MANAGER" => Role::RateManager,
        "MARKETING" => Role::Marketing,
        "VERIFICATION_REVIEWER" => Role::VerificationReviewer,
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

#[actix_web::test]
async fn finance_saves_a_limit_and_reads_it_back() {
    let db = TestDb::start().await;
    let a = save(&db, FINANCE, B01, json!("500")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["branch"], B01);
    assert_eq!(
        a.body["countDifferenceLimitThb"], "500.00",
        "two places: {}",
        a.text
    );
    assert_eq!(a.body["updatedBy"], user_id(FINANCE).as_str());
    assert!(a.body["updatedAt"].is_string(), "{}", a.text);

    let (org, limit, by, _) = db.row(B01).await.expect("one row");
    assert_eq!(org, ORG, "the branch's organisation");
    assert_eq!(limit.as_deref(), Some("500.00"));
    assert_eq!(by, user_id(FINANCE));

    let r = read(&db, FINANCE, B01).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        r.body["data"], a.body,
        "the view answers what the save answered"
    );
    db.finish().await;
}

/// §11.2: `CashLimitsManage` (`FINANCE` `FULL`) is held by `EXECUTIVE` and `FINANCE`.
#[actix_web::test]
async fn executive_may_set_it_too_and_a_second_save_replaces_the_first() {
    let db = TestDb::start().await;
    save(&db, FINANCE, B01, json!("500.00")).await;
    let a = save(&db, EXECUTIVE, B01, json!("250.5")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["countDifferenceLimitThb"], "250.50");
    assert_eq!(a.body["updatedBy"], user_id(EXECUTIVE).as_str());
    assert_eq!(db.rows().await, 1, "an upsert, one row per branch");
    let r = read(&db, EXECUTIVE, B01).await;
    assert_eq!(r.body["data"]["countDifferenceLimitThb"], "250.50");

    // an organisation-wide EXECUTIVE sets another branch; each branch reads only its own row
    let other = save(&db, EXECUTIVE, B09, json!("7")).await;
    assert_eq!(other.status, StatusCode::OK, "{}", other.text);
    assert_eq!(
        read(&db, EXECUTIVE, B09).await.body["data"]["countDifferenceLimitThb"],
        "7.00"
    );
    assert_eq!(
        read(&db, EXECUTIVE, B01).await.body["data"]["countDifferenceLimitThb"],
        "250.50"
    );
    assert_eq!(db.rows().await, 2);
    db.finish().await;
}

#[actix_web::test]
async fn null_removes_the_limit_so_every_difference_goes_to_finance() {
    let db = TestDb::start().await;
    save(&db, FINANCE, B01, json!("500")).await;
    let a = save(&db, FINANCE, B01, Value::Null).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["countDifferenceLimitThb"], Value::Null);
    let (_, limit, _, _) = db.row(B01).await.expect("the row stays");
    assert_eq!(limit, None, "stored NULL");
    db.finish().await;
}

#[actix_web::test]
async fn zero_is_a_limit_not_an_absence() {
    let db = TestDb::start().await;
    let a = save(&db, FINANCE, B01, json!("0")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["countDifferenceLimitThb"], "0.00");
    db.finish().await;
}

#[actix_web::test]
async fn a_branch_without_a_row_reads_as_no_limit() {
    let db = TestDb::start().await;
    let r = read(&db, FINANCE, B01).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        r.body["data"],
        json!({ "branch": B01, "countDifferenceLimitThb": null, "updatedBy": null, "updatedAt": null })
    );
    db.finish().await;
}

/// §11.2: roles without `CashLimitsManage` in `RoleGrants::seed(role)` may neither save nor read.
#[actix_web::test]
async fn roles_without_cash_limits_manage_may_neither_save_nor_read() {
    let db = TestDb::start().await;
    let candidates = [
        ("EXECUTIVE@*", Role::Executive),
        ("ADMIN@*", Role::Admin),
        ("RATE_MANAGER@XC-B01", Role::RateManager),
        ("BRANCH_MANAGER@XC-B01", Role::BranchManager),
        ("TELLER@XC-B01", Role::Teller),
        ("FINANCE@XC-B01", Role::Finance),
        ("MARKETING@XC-B01", Role::Marketing),
        ("VERIFICATION_REVIEWER@XC-B01", Role::VerificationReviewer),
    ];
    assert!(
        candidates
            .map(|(_, r)| r)
            .iter()
            .any(|r| !RoleGrants::seed(*r).contains(&Permission::CashLimitsManage)),
        "§11.2 must leave some role without CashLimitsManage, or this test refuses nobody"
    );
    for (who, _) in candidates
        .into_iter()
        .filter(|(_, r)| !RoleGrants::seed(*r).contains(&Permission::CashLimitsManage))
    {
        let a = save(&db, who, B01, json!("100")).await;
        assert_ne!(a.status, StatusCode::OK, "{who} save: {}", a.text);
        assert_eq!(code(&a), "roleNotAllowedHere", "{who} save: {}", a.text);
        let r = read(&db, who, B01).await;
        assert_ne!(r.status, StatusCode::OK, "{who} read: {}", r.text);
        assert_eq!(code(&r), "roleNotAllowedHere", "{who} read: {}", r.text);
    }
    assert_eq!(db.rows().await, 0, "nothing written");
    db.finish().await;
}

#[actix_web::test]
async fn a_bad_amount_is_refused_and_nothing_is_written() {
    let db = TestDb::start().await;
    for bad in [json!("-1"), json!("1.234"), json!("abc"), json!("")] {
        let a = save(&db, FINANCE, B01, bad.clone()).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{bad}: {}", a.text);
        assert_eq!(code(&a), "validationFailed", "{bad}: {}", a.text);
    }
    // money is a decimal string on the wire; a JSON number does not parse at all (the seam's answer)
    let n = save(&db, FINANCE, B01, json!(500)).await;
    assert_eq!(n.status, StatusCode::BAD_REQUEST, "{}", n.text);
    assert_eq!(db.rows().await, 0);
    db.finish().await;
}

#[actix_web::test]
async fn a_branch_outside_the_callers_scope_is_out_of_scope() {
    let db = TestDb::start().await;
    let a = save(&db, FINANCE, B09, json!("100")).await;
    assert_eq!(code(&a), "outOfScope", "{}", a.text);
    let r = read(&db, FINANCE, B09).await;
    assert_eq!(code(&r), "outOfScope", "{}", r.text);
    assert_eq!(db.rows().await, 0);
    db.finish().await;
}

#[actix_web::test]
async fn the_idempotency_key_is_required_and_a_repeat_writes_once() {
    let db = TestDb::start().await;
    let a = save_with(&db, FINANCE, B01, json!("100"), Key::Missing).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(db.rows().await, 0);

    let first = save_with(&db, FINANCE, B01, json!("100"), Key::This("k-1")).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let (_, _, _, at1) = db.row(B01).await.expect("written");
    let again = save_with(&db, FINANCE, B01, json!("100"), Key::This("k-1")).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.body, first.body, "the stored answer");
    let (_, _, _, at2) = db.row(B01).await.expect("still one row");
    assert_eq!(at1, at2, "not written again");

    let other = save_with(&db, FINANCE, B01, json!("999"), Key::This("k-1")).await;
    assert_eq!(code(&other), "idempotencyConflict", "{}", other.text);
    let (_, limit, _, _) = db.row(B01).await.expect("row");
    assert_eq!(limit.as_deref(), Some("100.00"), "unchanged");
    db.finish().await;
}
