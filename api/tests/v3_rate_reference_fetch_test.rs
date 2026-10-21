//! Ticket #599 (RE-2) — `POST /api/rate-references/fetch` writes a real reference batch from the
//! injected rate source, and answers that batch's id.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.2 ("Raw, as received", "Missing data", "Repeat
//! fetches", "Where the source comes from"), §4.7 (fetch answers "a batch id, new or the unchanged
//! one"), §9 RE-2. Before #599 the command builds `BotRateSource::bot()` itself in `register`, so a
//! test cannot give it a source and the route reaches the network.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
//!
//! Signatures this test assumes (#599 §7):
//! - `AppState::with_rate_source(self, source: Arc<dyn RateSource>) -> Self`
//!   (`api/src/bootstrap/mod.rs`), mirroring `with_database_probe`;
//! - `rate_reference_fetch.rs` `register` builds its writer from `state.rate_source`.
//!
//! The answer is `{"batchId": "<uuid>"}` (`RateReferenceFetchOutput`, camelCase).

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const FETCH: &str = "/api/rate-references/fetch";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
                "TEST_DATABASE_URL is not set.\n\
                 The reference-fetch route tests need a real PostgreSQL and do not skip. Point it \
                 at an existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("v3reffetch");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
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
            .expect("apply api/migrations to the test schema");
        Self { pool, schema }
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// `(provider, status)` of the batch `id`, if it exists.
    async fn batch(&self, id: &str) -> Option<(String, String)> {
        let id = Uuid::parse_str(id).unwrap_or_else(|e| panic!("batchId {id:?} is a uuid: {e}"));
        sqlx::query_as("SELECT provider, status FROM rate_reference_batches WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .expect("read the batch")
    }

    /// `(currency, reference_type, unit_size, buy, sell)` of every line of `batch`, by currency.
    async fn lines(
        &self,
        batch: &str,
    ) -> Vec<(String, String, Decimal, Option<Decimal>, Option<Decimal>)> {
        let id = Uuid::parse_str(batch).expect("a uuid");
        sqlx::query_as(
            "SELECT trim(currency_code), reference_type, unit_size, buy, sell \
             FROM rate_reference_lines WHERE batch_id = $1 ORDER BY currency_code",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("read the batch's lines")
    }

    /// Drop the schema. Reached only by a test that passed.
    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ the owner, always signed in

struct OwnerSession;

impl SessionReader for OwnerSession {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move { test_callers::callers(Some(test_callers::owner("owner-1")), None) })
    }
}

// ------------------------------------------------------------ the injected rate source

/// A rate source that answers what the test set, counting its calls. It never touches the network.
struct FakeSource {
    rows: Mutex<Option<Vec<Value>>>,
    calls: AtomicUsize,
}

impl FakeSource {
    fn answering(rows: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            rows: Mutex::new(Some(rows)),
            calls: AtomicUsize::new(0),
        })
    }

    fn refusing() -> Arc<Self> {
        Arc::new(Self {
            rows: Mutex::new(None),
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RateSource for FakeSource {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let rows = self.rows.lock().expect("the fake's lock").clone();
        Box::pin(async move {
            match rows {
                Some(rows) => Ok(ReferenceRates {
                    source: "fake-bank".to_owned(),
                    fetched_at: "2026-10-09T03:00:00.000Z".to_owned(),
                    kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_owned(),
                    rows,
                }),
                None => Err(AppError::Domain("the bank gave nothing usable".to_owned())),
            }
        })
    }
}

/// One row as the BoT adapter emits it after #599: the bank's own unit and value.
fn row(code: &str, unit: u64, transfer: &str, selling: &str) -> Value {
    json!({
        "code": code,
        "date": "2026-10-08",
        "unit": unit,
        "buyingSight": transfer,
        "buyingTransfer": transfer,
        "selling": selling,
        "original": { "buyingTransfer": transfer, "selling": selling },
    })
}

/// BoT's six, with the yen per 100 as the bank quotes it.
fn six() -> Vec<Value> {
    vec![
        row("USD", 1, "32.1", "32.45"),
        row("EUR", 1, "37.2", "37.9"),
        row("GBP", 1, "43.5", "44.909"),
        row("JPY", 100, "21", "21.63"),
        row("CNY", 1, "4.5", "4.62"),
        row("AUD", 1, "23", "23.6"),
    ]
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// `POST /api/rate-references/fetch` as the owner, with a fresh idempotency key each time, on an
/// app whose `AppState` carries `source`.
async fn fetch(db: &TestDb, source: &Arc<FakeSource>) -> Answer {
    let source: Arc<dyn RateSource> = source.clone();
    let state =
        AppState::new(Arc::new(common::settings(&[])), db.pool.clone()).with_rate_source(source);
    let reader: Arc<dyn SessionReader> = Arc::new(OwnerSession);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(FETCH)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(json!({}).to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn batch_id(a: &Answer) -> String {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "the fetch is accepted: {}",
        a.text
    );
    a.body["batchId"]
        .as_str()
        .unwrap_or_else(|| panic!("the answer names a batchId: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ tests

#[tokio::test]
async fn a_fetch_answers_the_id_of_the_batch_it_wrote_from_the_injected_source() {
    let db = TestDb::start().await;
    let source = FakeSource::answering(six());

    let id = batch_id(&fetch(&db, &source).await);

    assert_eq!(
        source.calls(),
        1,
        "the route asks the injected source, once"
    );
    assert_eq!(
        db.batch(&id).await,
        Some(("BOT".to_owned(), "COMPLETE".to_owned())),
        "the answered id is a batch in rate_reference_batches"
    );
    let lines = db.lines(&id).await;
    let codes: Vec<&str> = lines.iter().map(|l| l.0.as_str()).collect();
    assert_eq!(codes, ["AUD", "CNY", "EUR", "GBP", "JPY", "USD"]);
    assert!(
        lines.iter().all(|l| l.1 == "BOT"),
        "every line is a BOT line: {lines:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_yen_is_stored_per_100_with_the_banks_values_and_buy_is_the_transfer_rate() {
    let db = TestDb::start().await;
    let source = FakeSource::answering(six());

    let id = batch_id(&fetch(&db, &source).await);

    let lines = db.lines(&id).await;
    let jpy = lines.iter().find(|l| l.0 == "JPY").expect("a JPY line");
    assert_eq!(jpy.2, dec("100"), "JPY keeps the bank's unit");
    assert_eq!(jpy.3, Some(dec("21")), "buy = buying transfer, unchanged");
    assert_eq!(jpy.4, Some(dec("21.63")), "sell = selling, unchanged");
    let usd = lines.iter().find(|l| l.0 == "USD").expect("a USD line");
    assert_eq!(
        (usd.2, usd.3, usd.4),
        (dec("1"), Some(dec("32.1")), Some(dec("32.45")))
    );
    db.finish().await;
}

#[tokio::test]
async fn a_repeat_fetch_of_the_same_rows_answers_the_same_batch_and_writes_nothing() {
    let db = TestDb::start().await;
    let source = FakeSource::answering(six());

    let first = batch_id(&fetch(&db, &source).await);
    let (batches, lines) = (
        db.count("rate_reference_batches").await,
        db.count("rate_reference_lines").await,
    );
    let second = batch_id(&fetch(&db, &source).await);

    assert_eq!(source.calls(), 2, "each fetch asks the source again");
    assert_eq!(second, first, "the same rows answer the same batch");
    assert_eq!(db.count("rate_reference_batches").await, batches);
    assert_eq!(db.count("rate_reference_lines").await, lines);
    db.finish().await;
}

#[tokio::test]
async fn a_source_missing_a_currency_gives_a_partial_batch_without_that_line() {
    let db = TestDb::start().await;
    let mut five = six();
    five.retain(|r| r["code"] != json!("AUD"));
    let source = FakeSource::answering(five);

    let id = batch_id(&fetch(&db, &source).await);

    assert_eq!(db.batch(&id).await.map(|b| b.1), Some("PARTIAL".to_owned()));
    let codes: Vec<String> = db.lines(&id).await.into_iter().map(|l| l.0).collect();
    assert_eq!(
        codes,
        ["CNY", "EUR", "GBP", "JPY", "USD"],
        "AUD is absent, never guessed"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_refusing_source_writes_no_batch_and_answers_rate_unavailable() {
    let db = TestDb::start().await;
    let source = FakeSource::refusing();

    let a = fetch(&db, &source).await;

    assert_eq!(source.calls(), 1);
    assert_ne!(
        a.status,
        StatusCode::OK,
        "a refused fetch is not a success: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"],
        json!("rateUnavailable"),
        "{}",
        a.text
    );
    assert_eq!(db.count("rate_reference_batches").await, 0);
    assert_eq!(db.count("rate_reference_lines").await, 0);
    db.finish().await;
}
