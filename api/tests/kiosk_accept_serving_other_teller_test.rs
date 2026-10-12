//! Tests for 108-Plaza/xchange#1476:
//! Teller A must not be able to accept a kiosk request whose queue ticket teller B is serving.
//!
//! Done criteria:
//! 1. B serves; A's accept answers 400 notYourTicket; the request stays WAITING.
//!    B's own accept, and a BRANCH_MANAGER's, still succeed.
//! 2. Revert check: with the arm back to {}, the test is red.

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
use uuid::Uuid;
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
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_A_ID: &str = "00000000-0000-0000-0000-000000000010";
const TELLER_B_ID: &str = "00000000-0000-0000-0000-000000000020";
const MANAGER_ID: &str = "00000000-0000-0000-0000-000000000030";

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
        let schema = unique("qu1476");
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
             VALUES \
             ($1::uuid, $4::uuid, 'Teller A', 'ACTIVE', 'test'), \
             ($2::uuid, $4::uuid, 'Teller B', 'ACTIVE', 'test'), \
             ($3::uuid, $4::uuid, 'Manager', 'ACTIVE', 'test')",
        )
        .bind(TELLER_A_ID)
        .bind(TELLER_B_ID)
        .bind(MANAGER_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert users");

        // Open counts and shifts for tellers and manager (CA-3 shifts per teller)
        let count_a = uuid::Uuid::new_v4();
        let count_b = uuid::Uuid::new_v4();
        let count_m = uuid::Uuid::new_v4();

        sqlx::query(
            "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES \
             ($1, $4::uuid, $5, 'OPEN', $6::uuid, now(), 'key-a'), \
             ($2, $4::uuid, $5, 'OPEN', $7::uuid, now(), 'key-b'), \
             ($3, $4::uuid, $5, 'OPEN', $8::uuid, now(), 'key-m')",
        )
        .bind(count_a)
        .bind(count_b)
        .bind(count_m)
        .bind(ORG)
        .bind(B01)
        .bind(TELLER_A_ID)
        .bind(TELLER_B_ID)
        .bind(MANAGER_ID)
        .execute(&pool)
        .await
        .expect("insert cash counts");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id, opened_by_count) \
             VALUES \
             ('shift-a', $1, 'OPEN', now(), 'test', 'req-open-a', $2::uuid, $5), \
             ('shift-b', $1, 'OPEN', now(), 'test', 'req-open-b', $3::uuid, $6), \
             ('shift-m', $1, 'OPEN', now(), 'test', 'req-open-m', $4::uuid, $7)",
        )
        .bind(B01)
        .bind(TELLER_A_ID)
        .bind(TELLER_B_ID)
        .bind(MANAGER_ID)
        .bind(count_a)
        .bind(count_b)
        .bind(count_m)
        .execute(&pool)
        .await
        .expect("insert shifts");

        // Counters for tellers and manager
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES \
             ($1::uuid, $2, $3::uuid, 'ACTIVE'), \
             ($1::uuid, $2, $4::uuid, 'ACTIVE'), \
             ($1::uuid, $2, $5::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG)
        .bind(B01)
        .bind(TELLER_A_ID)
        .bind(TELLER_B_ID)
        .bind(MANAGER_ID)
        .execute(&pool)
        .await
        .expect("insert counters");

        // Ensure branch_service_status exists and is open with active counters
        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 2, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 2, queue_version = 1",
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
    let (uid, role) = match role_part {
        "TELLER_A" => (TELLER_A_ID, Role::Teller),
        "TELLER_B" => (TELLER_B_ID, Role::Teller),
        "BRANCH_MANAGER" => (MANAGER_ID, Role::BranchManager),
        other => panic!("unknown role {other}"),
    };

    let branch = (target_part != "*").then(|| BranchId::new(target_part));
    Some(User::new(
        uid,
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

fn legacy_of(spec: &str) -> Sessions {
    match spec.split_once('@') {
        Some(("TELLER_A", branch)) => Sessions {
            owner: None,
            staff: Actor::staff(
                TELLER_A_ID.to_string(),
                OldRole::Front,
                BranchId::new(branch),
            ),
        },
        Some(("TELLER_B", branch)) => Sessions {
            owner: None,
            staff: Actor::staff(
                TELLER_B_ID.to_string(),
                OldRole::Front,
                BranchId::new(branch),
            ),
        },
        Some(("BRANCH_MANAGER", branch)) => Sessions {
            owner: None,
            staff: Actor::staff(
                MANAGER_ID.to_string(),
                OldRole::Manager,
                BranchId::new(branch),
            ),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

fn spec_of(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get(AS)
        .and_then(|v| v.to_str().ok().map(|s| s.to_owned()))
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            match spec {
                Some(s) => legacy_of(&s),
                None => Sessions {
                    owner: None,
                    staff: None,
                },
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            match spec {
                Some(s) => {
                    let leg = legacy_of(&s);
                    let user = user_of(&s);
                    Callers {
                        owner: leg.owner,
                        staff: leg.staff,
                        user: user.clone(),
                        staff_user: user,
                    }
                }
                None => Callers {
                    owner: None,
                    staff: None,
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

async fn create_waiting_kiosk_request(db: &TestDb) -> (String, String) {
    let quote_ans = call(
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
    assert_eq!(
        quote_ans.status,
        StatusCode::OK,
        "create quote: {}",
        quote_ans.text
    );
    let req_id = quote_ans.body["id"].as_str().expect("quote id").to_string();
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '10s' WHERE id = $1")
        .bind(&req_id)
        .execute(&db.pool)
        .await
        .expect("update created_at");

    let confirm_ans = call(
        db,
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

    let (ticket_id, ticket_status): (String, String) =
        sqlx::query_as("SELECT id::text, status FROM queue_tickets WHERE kiosk_request_id = $1")
            .bind(&req_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch initial ticket");
    assert_eq!(ticket_status, "WAITING", "ticket initially WAITING");

    (req_id, ticket_id)
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn kiosk_accept_when_other_teller_serving_is_refused_and_manager_succeeds() {
    let db = TestDb::start().await;

    // 1. Create a kiosk request and confirm it into WAITING state
    let (req_id, ticket_id) = create_waiting_kiosk_request(&db).await;

    // 2. Teller B calls and starts the ticket through queue commands (QU-7 ticketStart)
    //    so the ticket moves to SERVING by teller B, while the kiosk request stays WAITING.
    let call_key = Uuid::new_v4().to_string();
    let call_ans = call(
        &db,
        Some(&format!("TELLER_B@{B01}")),
        Method::POST,
        &format!("/api/queue/tickets/{ticket_id}/call"),
        Some(json!({ "branchId": B01 })),
        Some(&call_key),
    )
    .await;
    assert_eq!(
        call_ans.status,
        StatusCode::OK,
        "teller B call ticket: {}",
        call_ans.text
    );

    let start_key = Uuid::new_v4().to_string();
    let start_ans = call(
        &db,
        Some(&format!("TELLER_B@{B01}")),
        Method::POST,
        &format!("/api/queue/tickets/{ticket_id}/start"),
        Some(json!({ "branchId": B01 })),
        Some(&start_key),
    )
    .await;
    assert_eq!(
        start_ans.status,
        StatusCode::OK,
        "teller B start ticket: {}",
        start_ans.text
    );

    // Verify ticket is SERVING by TELLER_B
    let (status, counter_teller_id): (String, Option<String>) = sqlx::query_as(
        "SELECT status, counter_teller_id::text FROM queue_tickets WHERE id = $1::uuid",
    )
    .bind(&ticket_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch ticket after start");
    assert_eq!(status, "SERVING");
    assert_eq!(counter_teller_id.as_deref(), Some(TELLER_B_ID));

    // Verify kiosk request is still WAITING
    let req_status: String = sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
        .bind(&req_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch kiosk_requests status");
    assert_eq!(req_status, "WAITING");

    // 3. Teller A attempts to accept the kiosk request:
    //    Must be refused with 400 notYourTicket.
    let accept_a = call(
        &db,
        Some(&format!("TELLER_A@{B01}")),
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
        accept_a.status,
        StatusCode::BAD_REQUEST,
        "teller A accept must fail with 400: {}",
        accept_a.text
    );
    assert_eq!(
        accept_a.body["error"], "notYourTicket",
        "error code must be notYourTicket, got: {}",
        accept_a.text
    );

    // Verify request still stays WAITING
    let req_status_after_a: String =
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
            .bind(&req_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch kiosk_requests status after A");
    assert_eq!(
        req_status_after_a, "WAITING",
        "request must stay WAITING after refusal"
    );

    // 4. BRANCH_MANAGER accepts the kiosk request:
    //    Must succeed!
    let accept_m = call(
        &db,
        Some(&format!("BRANCH_MANAGER@{B01}")),
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
        accept_m.status,
        StatusCode::OK,
        "BRANCH_MANAGER accept must succeed: {}",
        accept_m.text
    );

    // Verify request transitioned to ACCEPTED
    let req_status_after_m: String =
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
            .bind(&req_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch kiosk_requests status after Manager");
    assert_eq!(
        req_status_after_m, "ACCEPTED",
        "request must transition to ACCEPTED"
    );

    db.finish().await;
}

#[actix_web::test]
async fn kiosk_accept_when_other_teller_serving_is_refused_and_owner_succeeds() {
    let db = TestDb::start().await;

    // 1. Create a kiosk request and confirm it into WAITING state
    let (req_id, ticket_id) = create_waiting_kiosk_request(&db).await;

    // 2. Teller B calls and starts the ticket through queue commands (QU-7 ticketStart)
    let call_key = Uuid::new_v4().to_string();
    let call_ans = call(
        &db,
        Some(&format!("TELLER_B@{B01}")),
        Method::POST,
        &format!("/api/queue/tickets/{ticket_id}/call"),
        Some(json!({ "branchId": B01 })),
        Some(&call_key),
    )
    .await;
    assert_eq!(
        call_ans.status,
        StatusCode::OK,
        "teller B call ticket: {}",
        call_ans.text
    );

    let start_key = Uuid::new_v4().to_string();
    let start_ans = call(
        &db,
        Some(&format!("TELLER_B@{B01}")),
        Method::POST,
        &format!("/api/queue/tickets/{ticket_id}/start"),
        Some(json!({ "branchId": B01 })),
        Some(&start_key),
    )
    .await;
    assert_eq!(
        start_ans.status,
        StatusCode::OK,
        "teller B start ticket: {}",
        start_ans.text
    );

    // 3. Teller A attempts to accept the kiosk request:
    //    Must be refused with 400 notYourTicket.
    let accept_a = call(
        &db,
        Some(&format!("TELLER_A@{B01}")),
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
        accept_a.status,
        StatusCode::BAD_REQUEST,
        "teller A accept must fail with 400: {}",
        accept_a.text
    );
    assert_eq!(
        accept_a.body["error"], "notYourTicket",
        "error code must be notYourTicket, got: {}",
        accept_a.text
    );

    // Verify request still stays WAITING
    let req_status_after_a: String =
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
            .bind(&req_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch kiosk_requests status after A");
    assert_eq!(
        req_status_after_a, "WAITING",
        "request must stay WAITING after refusal"
    );

    // 4. Teller B (the serving teller) accepts the kiosk request:
    //    Must succeed!
    let accept_b = call(
        &db,
        Some(&format!("TELLER_B@{B01}")),
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
        accept_b.status,
        StatusCode::OK,
        "teller B accept must succeed: {}",
        accept_b.text
    );

    // Verify request transitioned to ACCEPTED
    let req_status_after_b: String =
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
            .bind(&req_id)
            .fetch_one(&db.pool)
            .await
            .expect("fetch kiosk_requests status after B");
    assert_eq!(
        req_status_after_b, "ACCEPTED",
        "request must transition to ACCEPTED"
    );

    db.finish().await;
}
