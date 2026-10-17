//! R-cash-limits up-front tests (#2054 row 10, re-point of #2039; design `docs/design/blueprint/bounded-inputs.md`
//! §4 R-re) — written before the implementation by claude/studio-claude-2 on the order of the xchange BE lane
//! controller claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `PUT /api/cash-limits` keeps its answers when its local check (`cash_limits_save.rs:33-53`) is replaced by
//! `v3::input::decimal` with `Bound::numeric(20, 2, Sign::NonNegative)`. These tests pin the answers that are the
//! same before and after: both sides of the `numeric(20,2)` bound (#2054 issuecomment-6201140913) and the refusal
//! shape. They are **green on main**; the re-point itself is checked by a grep in the ticket's done criteria.
//!
//! The harness is `cash_limits_overflow_test.rs`'s (#2039). Needs `TEST_DATABASE_URL`; nothing skips.

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
const FINANCE: &str = "FINANCE@XC-B01";

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
        let schema = unique("rcashlimitsbound");
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

enum Key {
    Fresh,
}

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key,
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
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn save(db: &TestDb, who: &str, branch: &str, limit: Value) -> Answer {
    call(
        db,
        who,
        Method::PUT,
        "/api/cash-limits",
        Some(json!({ "branch": branch, "countDifferenceLimitThb": limit })),
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

fn refused(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(code(a), "validationFailed", "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["field"], "countDifferenceLimitThb",
        "{what}: {}",
        a.text
    );
}

#[actix_web::test]
async fn the_largest_limit_the_column_holds_is_accepted_with_its_decimals() {
    let db = TestDb::start().await;
    let a = save(&db, FINANCE, B01, json!("999999999999999999.99")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        a.body["countDifferenceLimitThb"], "999999999999999999.99",
        "{}",
        a.text
    );
    let (_, limit, _, _) = db.row(B01).await.expect("stored row");
    assert_eq!(limit.as_deref(), Some("999999999999999999.99"));
    db.finish().await;
}

#[actix_web::test]
async fn ten_to_the_eighteen_is_refused_with_or_without_decimals_and_nothing_is_written() {
    let db = TestDb::start().await;
    for text in [
        "1000000000000000000",
        "1000000000000000000.00",
        "99999999999999999999",
    ] {
        let a = save(&db, FINANCE, B01, json!(text)).await;
        refused(&a, text);
    }
    assert_eq!(db.rows().await, 0, "nothing written");
    db.finish().await;
}

#[actix_web::test]
async fn a_third_decimal_a_negative_and_a_non_number_are_refused() {
    let db = TestDb::start().await;
    for text in ["1.234", "-1", "-0.01", "abc", ""] {
        let a = save(&db, FINANCE, B01, json!(text)).await;
        refused(&a, text);
    }
    assert_eq!(db.rows().await, 0, "nothing written");
    db.finish().await;
}

#[actix_web::test]
async fn an_ordinary_limit_and_zero_are_accepted() {
    let db = TestDb::start().await;
    let a = save(&db, FINANCE, B01, json!("500.50")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["countDifferenceLimitThb"], "500.50", "{}", a.text);
    let a = save(&db, FINANCE, B01, json!("0")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["countDifferenceLimitThb"], "0.00", "{}", a.text);
    db.finish().await;
}
