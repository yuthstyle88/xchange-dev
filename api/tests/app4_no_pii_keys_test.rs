//! Ticket #1610: A restored fallback PII key in the app-session path would pass every test
//! — add a no-keys test binary (APP-4 follow-up).
//!
//! Done criteria (§4.1):
//! The binary has one test. An anonymous IssueAndLimit save (the same request
//! app4_anonymous_retry_session_test sends) answers 500 with error code serverError
//! and no Set-Cookie header.
//!
//! Regression guard: NO PII keys are installed anywhere in this binary.
//! If a compiled-in fallback key is reintroduced to `current_pii_keys()`,
//! this test will flip from 500 to 200 (with Set-Cookie) and fail.

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

use xc_app::domain::app_session::APP_SESSION_COOKIE;
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
        let schema = unique("app4_no_pii_keys");
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

        // CRITICAL: We deliberately do NOT install any PII keys here.
        // The process must run with no PII keys installed.

        let room = Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        };

        room.publish_usd().await;
        room
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

// ------------------------------------------------------------ callers & session reader

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            Callers {
                user: None,
                staff_user: None,
            }
        })
    }
}

// ------------------------------------------------------------ HTTP test client

struct Answer {
    status: StatusCode,
    set_cookie: Vec<String>,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }
}

async fn call(room: &Room, method: Method, uri: &str, body: Option<(Value, &str)>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"));

    if let Some((b, key)) = body {
        req = req
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(b.to_string());
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let set_cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer {
        status,
        set_cookie,
        text,
        body,
    }
}

fn extract_app_sid(set_cookie: &[String]) -> Option<String> {
    set_cookie.iter().find_map(|c| {
        for part in c.split(';') {
            let trimmed = part.trim();
            if let Some(val) = trimmed.strip_prefix(&format!("{APP_SESSION_COOKIE}=")) {
                return Some(val.to_owned());
            }
        }
        None
    })
}

// ------------------------------------------------------------ regression test

/// An anonymous IssueAndLimit save, with NO PII keys installed anywhere in the binary,
/// answers 500 serverError with no Set-Cookie header.
#[actix_web::test]
async fn anonymous_issue_and_limit_without_pii_keys_answers_500_server_error_and_no_cookie() {
    let room = Room::start().await;
    let idem_key = unique("idem_no_pii");

    let body = json!({
        "branch": MAIN,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "amount": { "quantity": "500.0000" },
        "expectedVersion": null,
    });

    let res = call(
        &room,
        Method::POST,
        "/api/app/planner-drafts",
        Some((body, &idem_key)),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "must answer 500 Internal Server Error when no PII keys are installed, got: {}",
        res.text
    );
    assert_eq!(
        res.code(),
        "serverError",
        "error code must be serverError, got: {}",
        res.text
    );
    assert!(
        res.set_cookie.is_empty(),
        "must not set any cookie when no PII keys are installed, got Set-Cookie: {:?}",
        res.set_cookie
    );
    assert!(
        extract_app_sid(&res.set_cookie).is_none(),
        "must not issue an app session cookie"
    );

    room.finish().await;
}
