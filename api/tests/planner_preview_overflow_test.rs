//! The planner preview refuses a quantity whose THB overflows `Decimal`, instead of panicking
//! (tracking #2063, K-a caller migration, `xc-app/src/application/planner/planner_preview.rs`).
//!
//! Written before the implementation by the ticket author (claude/d7aa64). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `planner_preview` is public (an anonymous visitor reaches it). Its `amount.quantity` is any
//! positive decimal (`PlannerAmount::to_domain` in `presentation/dto/v3/planner.rs` builds it with
//! `ForeignAmount::from_decimal`, no upper bound), and `PlannerPreview::estimate` prices it with
//! `ExchangeArithmetic::thb(quantity, rate, unit_size)`. A quantity of 1e28 at the dollar's
//! 34.00 is 3.4e29, past `Decimal::MAX` (about 7.9e28), so `thb` panics "Multiplication overflowed"
//! (rust_decimal `arithmetic_impls.rs`). Today that panic unwinds out of the handler: in this
//! harness the test thread panics; on a server the worker drops the connection unanswered.
//!
//! After the fix (`try_thb`, overflow -> `CustomerAppError::ValidationFailed { field: "quantity" }`,
//! the error the same function already gives a quantity it cannot price): 400 `validationFailed`
//! on `quantity`, and an ordinary preview is unchanged.
//!
//! The harness (schema per test, the USD line 33.50 / 34.00 per 1, the route table) is a copy of
//! `app4_planner_routes_test.rs`'s, which this file does not edit.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database harness

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("app4_routes");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
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
            .expect("apply api/migrations");

        common::install_test_pii_keys();

        let room = Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        };

        // Seed USD rate publication for planner_preview
        room.publish_usd().await;

        room
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn publish_usd(&self) {
        let approval: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
             RETURNING id",
        )
        .bind(MAIN)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approved request for publication");

        let publication: Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, 1, now(), $2, 'test') RETURNING id",
        )
        .bind(MAIN)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("rate publication");

        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES (gen_random_uuid(), $1, 'USD', 1, 33.50, 34.00, $2)",
        )
        .bind(publication)
        .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
        .execute(&self.pool)
        .await
        .expect("published USD line");

        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(MAIN)
            .execute(&self.pool)
            .await
            .expect("make publication current");
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ an anonymous caller

struct Anonymous;

impl SessionReader for Anonymous {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            Callers {
                user: None,
                staff_user: None,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// `POST /api/v3/compute/planner_preview` with no session, as an anonymous visitor.
async fn preview(room: &Room, side: &str, quantity: &str, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Anonymous);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": side,
        "amount": { "quantity": quantity },
    });
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri("/api/v3/compute/planner_preview")
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", key.to_owned()))
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// 10^28: times 34.00 (or 33.50) it is past `Decimal::MAX`.
const OVERFLOWING_QUANTITY: &str = "10000000000000000000000000000";

fn assert_refused_on_quantity(answer: &Answer, label: &str) {
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "{label}: {}",
        answer.text
    );
    assert_eq!(
        answer.body["error"]["code"], "validationFailed",
        "{label}: {}",
        answer.text
    );
    assert_eq!(
        answer.body["error"]["field"], "quantity",
        "{label}: {}",
        answer.text
    );
}

// ------------------------------------------------------------ up-front tests

/// Row 1: a customer buying 10^28 dollars (the shop sells at 34.00) overflows quantity × rate.
/// Refused as an invalid quantity: 400 `validationFailed` on `quantity`, no panic, no 500.
#[actix_web::test]
async fn a_buy_quantity_whose_thb_overflows_is_refused_on_quantity() {
    let room = Room::start().await;
    let answer = preview(&room, "CUSTOMER_BUY", OVERFLOWING_QUANTITY, "overflow-buy").await;
    assert_refused_on_quantity(&answer, "CUSTOMER_BUY 1e28");
    room.finish().await;
}

/// Row 2: the same quantity sold to the shop (the shop buys at 33.50) is refused alike.
#[actix_web::test]
async fn a_sell_quantity_whose_thb_overflows_is_refused_on_quantity() {
    let room = Room::start().await;
    let answer = preview(
        &room,
        "CUSTOMER_SELL",
        OVERFLOWING_QUANTITY,
        "overflow-sell",
    )
    .await;
    assert_refused_on_quantity(&answer, "CUSTOMER_SELL 1e28");
    room.finish().await;
}

/// Row 3: the preview writes nothing when it refuses (it never writes; the refusal must not
/// change that).
#[actix_web::test]
async fn a_refused_preview_writes_no_rows() {
    let room = Room::start().await;
    let quotes = room.count("SELECT count(*) FROM quotes").await;
    let bookings = room.count("SELECT count(*) FROM bookings").await;
    let answer = preview(&room, "CUSTOMER_BUY", OVERFLOWING_QUANTITY, "overflow-rows").await;
    assert_refused_on_quantity(&answer, "CUSTOMER_BUY 1e28");
    assert_eq!(room.count("SELECT count(*) FROM quotes").await, quotes);
    assert_eq!(room.count("SELECT count(*) FROM bookings").await, bookings);
    room.finish().await;
}

/// Row 4 (keep-pin): an ordinary preview is unchanged. 100 USD: the customer buys at the
/// shop's 34.00 = 3,400.00 THB; the customer sells at the shop's 33.50 = 3,350.00 THB.
#[actix_web::test]
async fn an_ordinary_preview_is_unchanged() {
    let room = Room::start().await;
    let buy = preview(&room, "CUSTOMER_BUY", "100", "ordinary-buy").await;
    assert_eq!(buy.status, StatusCode::OK, "{}", buy.text);
    assert_eq!(buy.body["thb"], "3400.00", "{}", buy.text);
    let sell = preview(&room, "CUSTOMER_SELL", "100", "ordinary-sell").await;
    assert_eq!(sell.status, StatusCode::OK, "{}", sell.text);
    assert_eq!(sell.body["thb"], "3350.00", "{}", sell.text);
    room.finish().await;
}
