//! Specification tests for QU-8 (docs/design/blueprint/queue-tickets.md QU-8, §6, §8 M2, issue #1267):
//! Retire old queue reads (kiosk_notifications) and read work times from queue tickets.
//!
//! Written before implementation by test-writer gemini/pool-6 (ordered by claude/ad05bf8d, Owner's permission).
//! Do not edit, delete or weaken.
//!
//! Needs TEST_DATABASE_URL on a local _test database. Nothing here skips.

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
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xc_kiosk::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
const TELLER_ID_2: &str = "00000000-0000-0000-0000-000000000020";

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
        let schema = unique("qu8test");
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

        // Seed B01
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) VALUES ($1, 'Branch 1', $2::uuid)",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert B01");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2::uuid, 'Teller 1', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert teller 1");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid)",
        )
        .bind(B01)
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert shift 1");

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
        .expect("insert counter 1");

        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 1, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("upsert branch_service_status B01");

        rates_re4c_common::set_counter_rate(&pool, B01, "USD", "35.0", "36.0").await;

        // Seed B02
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) VALUES ($1, 'Branch 2', $2::uuid)",
        )
        .bind(B02)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert B02");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2::uuid, 'Teller 2', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID_2)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert teller 2");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-2', $1, 'OPEN', now(), 'test', 'req-open-2', $2::uuid)",
        )
        .bind(B02)
        .bind(TELLER_ID_2)
        .execute(&pool)
        .await
        .expect("insert shift 2");

        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG)
        .bind(B02)
        .bind(TELLER_ID_2)
        .execute(&pool)
        .await
        .expect("insert counter 2");

        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, true, 1, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1",
        )
        .bind(B02)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("upsert branch_service_status B02");

        rates_re4c_common::set_counter_rate(&pool, B02, "USD", "35.0", "36.0").await;

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
    let teller_id = if target_part == B02 {
        TELLER_ID_2
    } else {
        TELLER_ID
    };
    Some(User::new(
        teller_id,
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
        Some(("TELLER", branch)) => {
            let teller_id = if branch == B02 {
                TELLER_ID_2
            } else {
                TELLER_ID
            };
            Sessions {
                owner: None,
                staff: Actor::staff(teller_id.to_string(), OldRole::Front, BranchId::new(branch)),
            }
        }
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
                    let _leg = legacy_of(&s);
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

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

async fn create_quote(db: &TestDb, branch: &str, currency: &str, amount: i64) -> String {
    let ans = call(
        db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "quote",
            "payload": {
                "type": "BUY",
                "currency": currency,
                "amount": amount,
                "branchId": branch,
            },
        })),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "create quote: {}", ans.text);
    let id = ans.body["id"].as_str().expect("quote id").to_string();
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '10s' WHERE branch_id = $1 AND id = $2")
        .bind(branch)
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("update created_at");
    id
}

async fn confirm_quote(db: &TestDb, branch: &str, req_id: &str) {
    let ans = call(
        db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "confirm",
            "payload": {
                "id": req_id,
                "branchId": branch,
            },
        })),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "confirm quote: {}", ans.text);
}

async fn accept_quote(db: &TestDb, branch: &str, req_id: &str) {
    let ans = call(
        db,
        Some(&format!("TELLER@{branch}")),
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "accept",
            "payload": {
                "id": req_id,
                "branchId": branch,
            },
        })),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "accept quote: {}", ans.text);
}

// ------------------------------------------------------------ §9 Behaviour Table Tests

//// Row 1: Request confirmed (ticket WAITING), not accepted -> startedAt=null, completedAt=null
#[actix_web::test]
async fn row_1_confirmed_waiting_request_has_null_work_times() {
    let db = TestDb::start().await;
    let req_id = create_quote(&db, B01, "USD", 100).await;
    confirm_quote(&db, B01, &req_id).await;

    // Distinguish ticket value (null) from column value (stamped):
    // Setting column timestamps simulates legacy column values; the ticket is WAITING.
    let sql = format!(
        "DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='kiosk_requests' AND column_name='started_at') THEN \
         EXECUTE 'UPDATE kiosk_requests SET started_at = ''2026-01-01 10:00:00Z'', completed_at = ''2026-01-01 10:05:00Z'' WHERE branch_id = ''{B01}'' AND id = ''{req_id}'''; \
         END IF; END $$;"
    );
    sqlx::query(AssertSqlSafe(sql))
        .execute(&db.pool)
        .await
        .unwrap();

    let ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::GET,
        "/api/v3/views/kiosk_queue?state=waiting",
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "kiosk_queue: {}", ans.text);
    let rows = ans.body["data"].as_array().expect("array of rows");
    let row = rows.iter().find(|r| r["id"] == req_id).expect("found row");

    assert!(
        row["startedAt"].is_null(),
        "startedAt must be null for WAITING ticket, got: {:?}",
        row["startedAt"]
    );
    assert!(
        row["completedAt"].is_null(),
        "completedAt must be null for WAITING ticket, got: {:?}",
        row["completedAt"]
    );

    db.finish().await;
}

// Row 2: Accepted: the ticket is started, so served_at is set -> startedAt = ticket served_at
#[actix_web::test]
async fn row_2_accepted_request_started_at_equals_ticket_served_at() {
    let db = TestDb::start().await;
    let req_id = create_quote(&db, B01, "USD", 100).await;
    confirm_quote(&db, B01, &req_id).await;
    accept_quote(&db, B01, &req_id).await;

    // Distinguish ticket value from request column value:
    let ticket_time = "2026-05-20T14:30:00.000Z";
    sqlx::query(
        "UPDATE queue_tickets SET served_at = $2::timestamptz WHERE branch_id = $1 AND kiosk_request_id = $3",
    )
    .bind(B01)
    .bind(ticket_time)
    .bind(&req_id)
    .execute(&db.pool)
    .await
    .unwrap();

    let ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::GET,
        "/api/v3/views/kiosk_queue?state=working",
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "kiosk_queue: {}", ans.text);
    let rows = ans.body["data"].as_array().expect("array of rows");
    let row = rows.iter().find(|r| r["id"] == req_id).expect("found row");

    assert_eq!(
        row["startedAt"],
        json!(ticket_time),
        "startedAt must equal ticket served_at, not request column"
    );

    db.finish().await;
}

// Row 3: Completed: the ticket is finished -> completedAt = ticket completed_at
#[actix_web::test]
async fn row_3_completed_request_completed_at_equals_ticket_completed_at() {
    let db = TestDb::start().await;
    let req_id = create_quote(&db, B01, "USD", 100).await;
    confirm_quote(&db, B01, &req_id).await;
    accept_quote(&db, B01, &req_id).await;

    sqlx::query("UPDATE kiosk_requests SET status = 'COMPLETED' WHERE branch_id = $1 AND id = $2")
        .bind(B01)
        .bind(&req_id)
        .execute(&db.pool)
        .await
        .unwrap();

    let ticket_completed = "2026-05-20T15:00:00.000Z";
    sqlx::query(
        "UPDATE queue_tickets SET status = 'COMPLETED', completed_at = $2::timestamptz WHERE branch_id = $1 AND kiosk_request_id = $3",
    )
    .bind(B01)
    .bind(ticket_completed)
    .bind(&req_id)
    .execute(&db.pool)
    .await
    .unwrap();

    let ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::GET,
        "/api/v3/views/kiosk_queue?state=completed",
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "kiosk_queue: {}", ans.text);
    let rows = ans.body["data"].as_array().expect("array of rows");
    let row = rows.iter().find(|r| r["id"] == req_id).expect("found row");

    assert_eq!(
        row["completedAt"],
        json!(ticket_completed),
        "completedAt must equal ticket completed_at, not request column"
    );

    db.finish().await;
}

// Row 4: A request with no ticket (pre-M1 data) -> both null, no error
#[actix_web::test]
async fn row_4_request_with_no_ticket_returns_null_work_times() {
    let db = TestDb::start().await;
    let req_id = "req-pre-m1-no-ticket";

    // Direct insertion with no queue_tickets row
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, rate, foreign_amount, thb, input_amount, payment_method) \
         VALUES ($1, $2, 'ACCEPTED', now(), $3, 'BUY', 'USD', 35.0, 100.0, 3500.0, 100.0, 'Cash')",
    )
    .bind(B01)
    .bind(req_id)
    .bind(TELLER_ID)
    .execute(&db.pool)
    .await
    .unwrap();

    // If legacy columns exist (on main prior to M2), simulate legacy column values
    let sql = format!(
        "DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='kiosk_requests' AND column_name='started_at') THEN \
         EXECUTE 'UPDATE kiosk_requests SET started_at = ''2026-01-01 10:00:00Z'', completed_at = ''2026-01-01 10:05:00Z'' WHERE branch_id = ''{B01}'' AND id = ''{req_id}'''; \
         END IF; END $$;"
    );
    sqlx::query(AssertSqlSafe(sql))
        .execute(&db.pool)
        .await
        .unwrap();

    let ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::GET,
        "/api/v3/views/kiosk_queue?state=working",
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "kiosk_queue: {}", ans.text);
    let rows = ans.body["data"].as_array().expect("array of rows");
    let row = rows.iter().find(|r| r["id"] == req_id).expect("found row");

    assert!(
        row["startedAt"].is_null(),
        "startedAt must be null when there is no ticket, got: {:?}",
        row["startedAt"]
    );
    assert!(
        row["completedAt"].is_null(),
        "completedAt must be null when there is no ticket, got: {:?}",
        row["completedAt"]
    );

    db.finish().await;
}

// Row 5: Two branches, same request id pattern -> each request reads only its own branch's ticket
#[actix_web::test]
async fn row_5_two_branches_each_reads_only_own_branch_ticket() {
    let db = TestDb::start().await;
    let req_id = "req-shared-pattern";

    for (b, teller) in [(B01, TELLER_ID), (B02, TELLER_ID_2)] {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, rate, foreign_amount, thb, input_amount, payment_method) \
             VALUES ($1, $2, 'ACCEPTED', now(), $3, 'BUY', 'USD', 35.0, 100.0, 3500.0, 100.0, 'Cash')",
        )
        .bind(b)
        .bind(req_id)
        .bind(teller)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    let b1_ticket_time = "2026-07-01T10:00:00.000Z";
    let b2_ticket_time = "2026-07-02T10:00:00.000Z";

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, idempotency_key) \
         VALUES (gen_random_uuid(), $1::uuid, $2, current_date, 'WALK_IN', 1, 'A001', 'SERVING', $3, $4::uuid, now(), $5::timestamptz, 'key-b1')",
    )
    .bind(ORG)
    .bind(B01)
    .bind(req_id)
    .bind(TELLER_ID)
    .bind(b1_ticket_time)
    .execute(&db.pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, idempotency_key) \
         VALUES (gen_random_uuid(), $1::uuid, $2, current_date, 'WALK_IN', 1, 'A001', 'SERVING', $3, $4::uuid, now(), $5::timestamptz, 'key-b2')",
    )
    .bind(ORG)
    .bind(B02)
    .bind(req_id)
    .bind(TELLER_ID_2)
    .bind(b2_ticket_time)
    .execute(&db.pool)
    .await
    .unwrap();

    // Query B01
    let ans1 = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::GET,
        "/api/v3/views/kiosk_queue?state=working",
        None,
    )
    .await;
    assert_eq!(ans1.status, StatusCode::OK);
    let rows1 = ans1.body["data"].as_array().expect("b1 rows");
    let row1 = rows1.iter().find(|r| r["id"] == req_id).expect("b1 row");
    assert_eq!(
        row1["startedAt"],
        json!(b1_ticket_time),
        "B01 must read its own ticket served_at"
    );

    // Query B02
    let ans2 = call(
        &db,
        Some(&format!("TELLER@{B02}")),
        Method::GET,
        "/api/v3/views/kiosk_queue?state=working",
        None,
    )
    .await;
    assert_eq!(ans2.status, StatusCode::OK);
    let rows2 = ans2.body["data"].as_array().expect("b2 rows");
    let row2 = rows2.iter().find(|r| r["id"] == req_id).expect("b2 row");
    assert_eq!(
        row2["startedAt"],
        json!(b2_ticket_time),
        "B02 must read its own ticket served_at"
    );

    db.finish().await;
}

// Row 6: v2 accept answer (accept_claimed_json path) for an accepted request -> startedAt equals ticket served_at
#[actix_web::test]
async fn row_6_v2_accept_answer_started_at_equals_ticket_served_at() {
    let db = TestDb::start().await;
    let req_id = create_quote(&db, B01, "USD", 100).await;
    confirm_quote(&db, B01, &req_id).await;
    accept_quote(&db, B01, &req_id).await;

    let ticket_time = "2026-05-15T08:30:00.000Z";
    sqlx::query(
        "UPDATE queue_tickets SET served_at = $2::timestamptz WHERE branch_id = $1 AND kiosk_request_id = $3",
    )
    .bind(B01)
    .bind(ticket_time)
    .bind(&req_id)
    .execute(&db.pool)
    .await
    .unwrap();

    // 1. Through HTTP accept on already-accepted request (hits accept_mine_json / work_times_json)
    let ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "accept",
            "payload": {
                "id": req_id,
                "branchId": B01,
            }
        })),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "v2 accept call: {}", ans.text);
    assert_eq!(
        ans.body["startedAt"],
        json!(ticket_time),
        "v2 accept answer startedAt must equal ticket served_at"
    );

    // 2. Direct verification of times_in and accept_claimed_json
    let repo = PgKioskRepository::new(db.pool.clone());
    let mut tx = db.pool.begin().await.unwrap();
    let times = repo
        .times_in(&mut tx, &BranchId::new(B01), &req_id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let v2_json = xchange::presentation::dto::kiosk::accept_claimed_json(&times);
    let val = v2_json.to_value();
    assert_eq!(
        val["startedAt"],
        json!(ticket_time),
        "accept_claimed_json startedAt must equal ticket served_at"
    );

    db.finish().await;
}

// Row 7: GET /api/v3/views/kiosk_notifications -> 404
#[actix_web::test]
async fn row_7_kiosk_notifications_view_answers_404() {
    let db = TestDb::start().await;
    let ans = call(
        &db,
        Some(&format!("TELLER@{B01}")),
        Method::GET,
        "/api/v3/views/kiosk_notifications",
        None,
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "kiosk_notifications must be retired and answer 404, got: {} ({})",
        ans.status,
        ans.text
    );
    db.finish().await;
}

// Row 8: After the migration -> kiosk_requests has no started_at and no completed_at column
#[actix_web::test]
async fn row_8_after_migration_kiosk_requests_has_no_work_times_columns() {
    let db = TestDb::start().await;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = current_schema() \
           AND table_name = 'kiosk_requests' \
           AND column_name IN ('started_at', 'completed_at')",
    )
    .fetch_one(&db.pool)
    .await
    .expect("query columns count");

    assert_eq!(
        count, 0,
        "kiosk_requests must not have started_at or completed_at columns after M2 migration (found {count})"
    );
    db.finish().await;
}

// Row 9: The queue page order (kiosk_repository.rs:546-548 sorts by started_at) -> now by ticket's served_at, NULLS LAST then id
#[actix_web::test]
async fn row_9_queue_working_order_sorts_by_ticket_served_at_nulls_last() {
    let db = TestDb::start().await;

    // Seed 3 accepted requests
    let reqs = [
        (
            "req-order-1",
            "2026-01-01 10:00:00Z",
            Some("2026-05-01 14:00:00Z"),
        ),
        (
            "req-order-2",
            "2026-01-01 12:00:00Z",
            Some("2026-05-01 10:00:00Z"),
        ),
        ("req-order-3", "2026-01-01 08:00:00Z", None),
    ];

    for (id, col_started, ticket_served) in reqs {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, rate, foreign_amount, thb, input_amount, payment_method) \
             VALUES ($1, $2, 'ACCEPTED', now(), $3, 'BUY', 'USD', 35.0, 100.0, 3500.0, 100.0, 'Cash')",
        )
        .bind(B01)
        .bind(id)
        .bind(TELLER_ID)
        .execute(&db.pool)
        .await
        .unwrap();

        // If legacy column exists, set legacy started_at to make it fail on main
        let sql = format!(
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='kiosk_requests' AND column_name='started_at') THEN \
             EXECUTE 'UPDATE kiosk_requests SET started_at = ''{col_started}''::timestamptz WHERE branch_id = ''{B01}'' AND id = ''{id}'''; \
             END IF; END $$;"
        );
        sqlx::query(AssertSqlSafe(sql))
            .execute(&db.pool)
            .await
            .unwrap();

        if let Some(served) = ticket_served {
            sqlx::query(
                "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, idempotency_key) \
                 VALUES (gen_random_uuid(), $1::uuid, $2, current_date, 'WALK_IN', (SELECT COALESCE(MAX(sequence), 0) + 1 FROM queue_tickets), 'A001', 'SERVING', $3, $4::uuid, now(), $5::timestamptz, $6)",
            )
            .bind(ORG)
            .bind(B01)
            .bind(id)
            .bind(TELLER_ID)
            .bind(served)
            .bind(format!("key-{id}"))
            .execute(&db.pool)
            .await
            .unwrap();
        }
    }

    let repo = PgKioskRepository::new(db.pool.clone());
    let (working, _count) = repo.working(&BranchId::new(B01), 10).await.unwrap();
    let ids: Vec<&str> = working.iter().map(|w| w.id.as_str()).collect();

    // Under ticket served_at ASC NULLS LAST, id ASC:
    // req-order-2 (served 10:00) comes 1st
    // req-order-1 (served 14:00) comes 2nd
    // req-order-3 (served NULL) comes 3rd
    assert_eq!(
        ids,
        vec!["req-order-2", "req-order-1", "req-order-3"],
        "working requests must be sorted by ticket served_at ASC NULLS LAST, id ASC"
    );

    db.finish().await;
}
