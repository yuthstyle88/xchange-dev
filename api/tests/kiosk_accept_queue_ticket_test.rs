//! Specification tests for QU-6 (docs/design/blueprint/queue-tickets.md §4.5, §9.1 row QU-6):
//! kiosk `accept` on a WAITING request calls and starts its queue ticket.
//!
//! Written before implementation by test-writer (gemini/studio-11, ordered by
//! claude/5edf7bb5, Owner's Rule A). Do not edit, delete or weaken.
//!
//! Rule A requirement:
//! (a) `accept` on a WAITING request calls and starts its queue ticket:
//!     ticket moves to SERVING, counter_teller_id = staff user id, served_at is recorded,
//!     and queue_events records CALL and START events.
//!
//! Needs TEST_DATABASE_URL; nothing skips.

mod common;

#[path = "rates_re4c_common/mod.rs"]
mod rates_re4c_common;

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
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";

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
        let schema = unique("qu6test");
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

        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) VALUES ($1, 'Branch 1', $2::uuid)",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert branch");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2::uuid, 'Teller 1', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert teller user");

        // Shift for teller
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid)",
        )
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert shift");

        // Counter for teller
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG)
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert counter");

        // Ensure branch_service_status exists and is open with 1 active counter
        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 1, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("upsert branch_service_status");

        // Seed counter rate for USD per RE-7
        rates_re4c_common::set_counter_rate(&pool, B01, "USD", "35.0", "36.0").await;

        Self { pool, schema }
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

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role_part, target_part) = spec.split_once('@')?;
    let role = match role_part {
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };

    let branch = (target_part != "*").then(|| BranchId::new(target_part));
    Some(User::new(
        TELLER_ID,
        ORG,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(AS)
        .and_then(|v| v.to_str().ok().map(|s| s.to_owned()))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            match spec {
                Some(s) => {
                    let user = user_of(&s);
                    Callers {
                        user: user.clone(),
                        staff_user: user,
                    }
                }
                None => Callers {
                    user: None,
                    staff_user: None,
                },
            }
        })
    }
}

// ------------------------------------------------------------ HTTP harness

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(
    db: &TestDb,
    who: Option<&str>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Option<&str>,
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
        .insert_header((header::HOST, HOST));

    if let Some(who) = who {
        req = req.insert_header((AS, who));
    }

    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
    }

    if let Some(k) = key {
        req = req.insert_header(("idempotency-key", k));
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

async fn create_quote(db: &TestDb) -> String {
    let ans = call(
        db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "quote",
            "payload": {
                "type": "BUY",
                "currency": "USD",
                "amount": 100,
                "branchId": B01,
            },
        })),
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "create quote: {}", ans.text);
    let id = ans.body["id"].as_str().expect("quote id").to_string();
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '10s' WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("update created_at");
    id
}

// ------------------------------------------------------------ test

#[actix_web::test]
async fn kiosk_accept_on_waiting_request_calls_and_starts_queue_ticket() {
    let db = TestDb::start().await;

    // 1. Create a quote and confirm it into WAITING state
    let req_id = create_quote(&db).await;

    let confirm_ans = call(
        &db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "confirm",
            "payload": {
                "id": req_id,
                "branchId": B01,
            },
        })),
        None,
    )
    .await;
    assert_eq!(
        confirm_ans.status,
        StatusCode::OK,
        "confirm: {}",
        confirm_ans.text
    );

    // Verify ticket exists and is WAITING
    let (initial_status, initial_seq): (String, i32) =
        sqlx::query_as("SELECT status, sequence FROM queue_tickets WHERE kiosk_request_id = $1")
            .bind(&req_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch initial ticket");
    assert_eq!(initial_status, "WAITING", "ticket initially WAITING");
    assert_eq!(initial_seq, 1, "first ticket has sequence 1");

    // 2. Staff (teller) accepts the request
    let accept_ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "accept",
            "payload": {
                "id": req_id,
                "branchId": B01,
            },
        })),
        None,
    )
    .await;
    assert_eq!(
        accept_ans.status,
        StatusCode::OK,
        "accept: {}",
        accept_ans.text
    );

    // 3. QU-6 pin: accept starts the request's ticket, calling it first if it is WAITING.
    // Ticket status moves to SERVING, counter_teller_id = staff user id, served_at is recorded.
    let (status, teller_id, served_at_is_set): (String, Option<String>, bool) = sqlx::query_as(
        "SELECT status, counter_teller_id::text, served_at IS NOT NULL \
         FROM queue_tickets WHERE kiosk_request_id = $1",
    )
    .bind(&req_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch ticket after accept");

    assert_eq!(
        status, "SERVING",
        "accept on a WAITING request must transition its queue ticket to SERVING"
    );
    assert_eq!(
        teller_id.as_deref(),
        Some(TELLER_ID),
        "ticket counter_teller_id must be the accepting staff"
    );
    assert!(
        served_at_is_set,
        "served_at must be set on the queue ticket"
    );

    // 4. Verify queue_events contains CALL and START
    let events: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT kind, actor_id::text FROM queue_events \
         WHERE ticket_id = (SELECT id FROM queue_tickets WHERE kiosk_request_id = $1) \
         ORDER BY created_at",
    )
    .bind(&req_id)
    .fetch_all(&db.pool)
    .await
    .expect("fetch queue events");

    let kinds: Vec<&str> = events.iter().map(|(k, _)| k.as_str()).collect();
    assert!(
        kinds.contains(&"CALL"),
        "queue_events must contain CALL event, got: {kinds:?}"
    );
    assert!(
        kinds.contains(&"START"),
        "queue_events must contain START event, got: {kinds:?}"
    );

    db.finish().await;
}
