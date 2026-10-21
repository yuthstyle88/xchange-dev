//! Ticket #2649 — orgStartingPriceSave answers 500 when the same prices are saved twice in one day
//!
//! Behaviour table from issue #2649 §8:
//! 1. Save prices P with reason R1, then save the same P with reason R2 the same day (different idempotency keys):
//!    Both 200; two rate_reference_batches rows; the second save's reason is recorded as any save's is.
//! 2. Save P, then save P′ (different prices) the same day:
//!    Both 200 (kept).
//! 3. The same request replayed with the same idempotency key:
//!    The stored first answer, nothing new written (kept, command wrapper).

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

const CMD_SAVE: &str = "/api/v3/commands/org_starting_price_save";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
    org: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("sp1sameday");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to test schema");

        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02'), ('XC-B09', 'B09')",
        )
        .execute(&pool)
        .await
        .expect("insert branches");

        let org: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'XC-B01'")
                .fetch_one(&pool)
                .await
                .expect("the branches' organisation");

        Self { pool, schema, org }
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers {
    org: String,
}

fn user_id(spec: &str) -> String {
    let n = match spec {
        "owner" | "EXECUTIVE@*" => 1,
        "admin" | "ADMIN@*" => 2,
        "rates" | "RATES@XC-B01" => 3,
        "teller" | "TELLER@XC-B01" => 4,
        "branch_manager" | "BRANCH_MANAGER@XC-B01" => 5,
        other => panic!("no user id written for {other}"),
    };
    format!("00000000-0000-4000-8000-{n:012}")
}

fn user_of(spec: &str, org: &str) -> Option<AccessActor> {
    let (role, branch) = match spec {
        "owner" => ("EXECUTIVE", "*"),
        "admin" => ("ADMIN", "*"),
        "rates" => ("RATES", "XC-B01"),
        "teller" => ("TELLER", "XC-B01"),
        "branch_manager" => ("BRANCH_MANAGER", "XC-B01"),
        other => other.split_once('@')?,
    };
    let role = match role {
        "EXECUTIVE" => AccessRole::Executive,
        "ADMIN" => AccessRole::Admin,
        "RATES" | "RATE_MANAGER" => AccessRole::RateManager,
        "TELLER" => AccessRole::Teller,
        "BRANCH_MANAGER" => AccessRole::BranchManager,
        "FINANCE" => AccessRole::Finance,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(AccessActor::new(
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

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    call_with_key(db, who, method, uri, body, unique("key")).await
}

async fn call_with_key(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    idem_key: String,
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
            .insert_header(("idempotency-key", idem_key.as_str()))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn entry(currency: &str, buy: Option<&str>, sell: Option<&str>) -> Value {
    json!({
        "currency": currency,
        "buy": buy,
        "sell": sell,
    })
}

fn save_payload(prices: Vec<Value>, reason: &str) -> Value {
    json!({
        "prices": prices,
        "reason": reason,
    })
}

// ------------------------------------------------------------ behaviour table tests

/// Behaviour table row 1:
/// Save prices P with reason R1, then save the same P with reason R2 the same day (different idempotency keys)
/// -> Both 200; two rate_reference_batches rows; the second save's reason is recorded as any save's is
#[actix_web::test]
async fn row1_save_same_prices_different_reason_same_day_succeeds() {
    let db = TestDb::start().await;

    let p1 = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "initial starting price",
    );
    let a1 = call_with_key(
        &db,
        "owner",
        Method::POST,
        CMD_SAVE,
        Some(p1),
        unique("key1"),
    )
    .await;
    assert_eq!(
        a1.status,
        StatusCode::OK,
        "first save must succeed: {}",
        a1.text
    );

    let p2 = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "corrected reason for starting price",
    );
    let a2 = call_with_key(
        &db,
        "owner",
        Method::POST,
        CMD_SAVE,
        Some(p2),
        unique("key2"),
    )
    .await;
    assert_eq!(
        a2.status,
        StatusCode::OK,
        "second save of identical prices same day must succeed: {}",
        a2.text
    );

    // Verify rate_reference_batches: exactly 2 rows for START:<org>
    let batch_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches WHERE provider = $1")
            .bind(format!("START:{}", db.org))
            .fetch_one(&db.pool)
            .await
            .expect("count rate_reference_batches");
    assert_eq!(batch_count, 2, "must write two rate_reference_batches rows");

    // Verify org_starting_prices: 2 rows, version 1 has R1, version 2 has R2
    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT version, reason FROM org_starting_prices WHERE currency_code = 'USD' ORDER BY version ASC",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch org_starting_prices");
    assert_eq!(
        rows.len(),
        2,
        "must have two versions in org_starting_prices"
    );
    assert_eq!(rows[0], (1, "initial starting price".to_string()));
    assert_eq!(
        rows[1],
        (2, "corrected reason for starting price".to_string())
    );

    db.finish().await;
}

/// Behaviour table row 2:
/// Save P, then save P′ (different prices) the same day
/// -> Both 200 (kept)
#[actix_web::test]
async fn row2_save_different_prices_same_day_succeeds() {
    let db = TestDb::start().await;

    let p1 = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "initial starting price",
    );
    let a1 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p1)).await;
    assert_eq!(
        a1.status,
        StatusCode::OK,
        "first save must succeed: {}",
        a1.text
    );

    let p2 = save_payload(
        vec![entry("USD", Some("32.50000000"), Some("33.50000000"))],
        "updated starting price",
    );
    let a2 = call(&db, "owner", Method::POST, CMD_SAVE, Some(p2)).await;
    assert_eq!(
        a2.status,
        StatusCode::OK,
        "second save with different prices must succeed: {}",
        a2.text
    );

    let batch_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches WHERE provider = $1")
            .bind(format!("START:{}", db.org))
            .fetch_one(&db.pool)
            .await
            .expect("count rate_reference_batches");
    assert_eq!(batch_count, 2, "must write two rate_reference_batches rows");

    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT version, reason FROM org_starting_prices WHERE currency_code = 'USD' ORDER BY version ASC",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch org_starting_prices");
    assert_eq!(
        rows.len(),
        2,
        "must have two versions in org_starting_prices"
    );
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[1].0, 2);

    db.finish().await;
}

/// Behaviour table row 3:
/// The same request replayed with the same idempotency key
/// -> The stored first answer, nothing new written (kept, command wrapper)
#[actix_web::test]
async fn row3_replay_same_request_same_idempotency_key_returns_stored_answer_without_duplicate() {
    let db = TestDb::start().await;

    let idem_key = unique("replay_key");
    let payload = save_payload(
        vec![entry("USD", Some("32.00000000"), Some("33.00000000"))],
        "initial starting price",
    );

    let a1 = call_with_key(
        &db,
        "owner",
        Method::POST,
        CMD_SAVE,
        Some(payload.clone()),
        idem_key.clone(),
    )
    .await;
    assert_eq!(
        a1.status,
        StatusCode::OK,
        "first save must succeed: {}",
        a1.text
    );

    let a2 = call_with_key(
        &db,
        "owner",
        Method::POST,
        CMD_SAVE,
        Some(payload),
        idem_key,
    )
    .await;
    assert_eq!(
        a2.status,
        StatusCode::OK,
        "replayed save must succeed: {}",
        a2.text
    );
    assert_eq!(
        a1.body, a2.body,
        "replayed response must match stored first answer"
    );

    let batch_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches WHERE provider = $1")
            .bind(format!("START:{}", db.org))
            .fetch_one(&db.pool)
            .await
            .expect("count rate_reference_batches");
    assert_eq!(
        batch_count, 1,
        "exactly 1 rate_reference_batches row must be written"
    );

    let price_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM org_starting_prices WHERE currency_code = 'USD'")
            .fetch_one(&db.pool)
            .await
            .expect("count org_starting_prices");
    assert_eq!(
        price_count, 1,
        "exactly 1 org_starting_prices row must be written"
    );

    db.finish().await;
}
