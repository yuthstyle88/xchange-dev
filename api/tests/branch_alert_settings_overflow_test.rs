//! Ticket #2040: `branchAlertSettingsSave` floor `minQuantity` overflow (`numeric(20,4)`).
//!
//! Behaviour table (§4, §9):
//! 1. `"10000000000000000"` (17 digits) -> 400 `validationFailed`, field `floors`; no `branch_stock_floors` row written or changed.
//! 2. `"9999999999999999.9999"` -> 200, echoed back.
//! 3. `"10"` -> 200.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

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

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("ex5overflow");
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

    async fn settings_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM branch_alert_settings")
            .await
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

/// Row 1: `"10000000000000000"` (17 digits) -> 400 `validationFailed`, field `floors`;
/// no `branch_stock_floors` row written or changed.
#[actix_web::test]
async fn floor_min_quantity_of_seventeen_digits_is_refused_with_validation_failed_on_floors() {
    let db = TestDb::start().await;

    let a = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "10000000000000000")], None),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "17-digit minQuantity status; body: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"],
        json!("validationFailed"),
        "{}",
        a.text
    );
    assert_eq!(a.body["error"]["field"], json!("floors"), "{}", a.text);
    assert_eq!(db.settings_rows().await, 0);
    assert_eq!(db.floor_rows().await, 0);

    // Seed an existing floor and verify a 17-digit update does not change it.
    let seeded = save(&db, EXECUTIVE, body(B01, Some(30), &[("USD", "10")], None)).await;
    assert_eq!(seeded.status, StatusCode::OK, "{}", seeded.text);

    let b = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "10000000000000000")], Some(1)),
    )
    .await;
    assert_eq!(
        b.status,
        StatusCode::BAD_REQUEST,
        "17-digit update status; body: {}",
        b.text
    );
    assert_eq!(
        b.body["error"]["code"],
        json!("validationFailed"),
        "{}",
        b.text
    );
    assert_eq!(b.body["error"]["field"], json!("floors"), "{}", b.text);
    let stored: Decimal = sqlx::query_scalar(
        "SELECT min_quantity FROM branch_stock_floors WHERE branch_id = 'XC-B01' AND currency = 'USD'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read existing floor");
    assert_eq!(stored, Decimal::from(10));

    db.finish().await;
}

/// Row 2: `"9999999999999999.9999"` -> 200, echoed back.
#[actix_web::test]
async fn floor_min_quantity_at_sixteen_digits_and_four_decimals_is_accepted_and_echoed() {
    let db = TestDb::start().await;

    let a = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "9999999999999999.9999")], None),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "16-digit max status; body: {}",
        a.text
    );
    assert_set(
        &a.body,
        B01,
        Some(30),
        &[("USD", "9999999999999999.9999")],
        Some(1),
    );
    assert_eq!(db.floor_rows().await, 1);

    db.finish().await;
}

/// Row 3: `"10"` -> 200.
#[actix_web::test]
async fn floor_min_quantity_ten_is_accepted() {
    let db = TestDb::start().await;

    let a = save(&db, EXECUTIVE, body(B01, Some(30), &[("USD", "10")], None)).await;
    assert_eq!(a.status, StatusCode::OK, "10 status; body: {}", a.text);
    assert_set(&a.body, B01, Some(30), &[("USD", "10")], Some(1));
    assert_eq!(db.floor_rows().await, 1);

    db.finish().await;
}
