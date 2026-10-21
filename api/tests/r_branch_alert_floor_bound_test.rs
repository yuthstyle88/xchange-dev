//! R-branch-alert up-front tests (#2054 row 11, re-point of #2040; design `docs/design/blueprint/bounded-inputs.md`
//! §4 R-re) — written before the implementation by claude/studio-claude-2 on the order of the xchange BE lane
//! controller claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `branchAlertSettingsSave` keeps its answers for `floors[].minQuantity` when its local check
//! (`branch_alert_settings_save.rs:79-110`) is replaced by `v3::input::decimal("floors", …,
//! Bound::numeric(20, 4, Sign::NonNegative))`. These tests pin the answers that are the same before and after: both
//! sides of `numeric(20,4)`, the scale, the sign and the refusal shape (400 `validationFailed`, field `floors`). They
//! are **green on main**; the re-point itself is checked by a grep in the ticket's done criteria.
//!
//! The harness is `branch_alert_settings_overflow_test.rs`'s (#2040). Needs `TEST_DATABASE_URL`; nothing skips.

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
use rust_decimal::Decimal;
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
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const SAVE: &str = "/api/v3/commands/branch_alert_settings_save";
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

struct TestDb {
    pool: PgPool,
    schema: String,
    org: String,
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
        let schema = unique("rbranchalertbound");
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
            .max_connections(8)
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
        let org: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'XC-B01'")
                .fetch_one(&pool)
                .await
                .expect("the branch's organisation");
        Self { pool, schema, org }
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn floor_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM branch_stock_floors").await
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct HeaderUsers {
    org: String,
}

fn user_id(spec: &str) -> String {
    let n = match spec {
        EXECUTIVE => 1,
        other => panic!("no user id written for {other}"),
    };
    format!("00000000-0000-4000-8000-{n:012}")
}

fn user_of(spec: &str, org: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "EXECUTIVE" => Role::Executive,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        org,
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
        let org = self.org.clone();
        Box::pin(async move {
            let user = user_of(&spec, &org);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: &str,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers {
        org: db.org.clone(),
    });
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
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn save(db: &TestDb, who: &str, body: Value) -> Answer {
    call(db, who, Method::POST, SAVE, Some(body), &unique("key")).await
}

fn body(
    branch: &str,
    minutes: Option<i64>,
    floors: &[(&str, &str)],
    expected: Option<i64>,
) -> Value {
    json!({
        "branch": branch,
        "staleDataMinutes": minutes,
        "floors": floors.iter().map(|(c, q)| json!({ "currency": c, "minQuantity": q })).collect::<Vec<_>>(),
        "expectedVersion": expected,
    })
}

fn decimal(v: &Value) -> Decimal {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("minQuantity is a decimal string: {v}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("minQuantity {s:?} parses: {e}"))
}

fn assert_set(
    got: &Value,
    branch: &str,
    minutes: Option<i64>,
    floors: &[(&str, &str)],
    version: Option<i64>,
) {
    assert_eq!(got["branch"], json!(branch), "{got}");
    assert_eq!(got["staleDataMinutes"], json!(minutes), "{got}");
    assert_eq!(got["version"], json!(version), "{got}");
    let listed = got["floors"]
        .as_array()
        .unwrap_or_else(|| panic!("floors is an array: {got}"));
    assert_eq!(listed.len(), floors.len(), "{got}");
    for (currency, quantity) in floors {
        let floor = listed
            .iter()
            .find(|f| f["currency"] == json!(currency))
            .unwrap_or_else(|| panic!("a floor for {currency}: {got}"));
        assert_eq!(
            decimal(&floor["minQuantity"]),
            Decimal::from_str(quantity).expect("a test decimal"),
            "{currency}: {got}"
        );
    }
}

fn refused_on_floors(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["code"],
        json!("validationFailed"),
        "{what}: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"],
        json!("floors"),
        "{what}: {}",
        a.text
    );
}

#[actix_web::test]
async fn the_largest_floor_the_column_holds_is_accepted() {
    let db = TestDb::start().await;
    let a = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "9999999999999999.9999")], None),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_set(
        &a.body,
        B01,
        Some(30),
        &[("USD", "9999999999999999.9999")],
        Some(1),
    );
    db.finish().await;
}

#[actix_web::test]
async fn ten_to_the_sixteen_and_above_are_refused_and_nothing_is_written() {
    let db = TestDb::start().await;
    for q in [
        "10000000000000000",
        "10000000000000000.0000",
        "99999999999999999999",
    ] {
        let a = save(&db, EXECUTIVE, body(B01, Some(30), &[("USD", q)], None)).await;
        refused_on_floors(&a, q);
    }
    assert_eq!(db.floor_rows().await, 0, "nothing written");
    db.finish().await;
}

#[actix_web::test]
async fn a_fifth_decimal_a_negative_and_a_non_number_are_refused() {
    let db = TestDb::start().await;
    for q in ["0.00001", "-1", "-0.0001", "abc", ""] {
        let a = save(&db, EXECUTIVE, body(B01, Some(30), &[("USD", q)], None)).await;
        refused_on_floors(&a, q);
    }
    assert_eq!(db.floor_rows().await, 0, "nothing written");
    db.finish().await;
}

#[actix_web::test]
async fn zero_and_four_decimals_are_accepted() {
    let db = TestDb::start().await;
    let a = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "0"), ("JPY", "0.0001")], None),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_set(
        &a.body,
        B01,
        Some(30),
        &[("USD", "0"), ("JPY", "0.0001")],
        Some(1),
    );
    db.finish().await;
}
