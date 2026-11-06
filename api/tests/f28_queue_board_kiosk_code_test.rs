//! Tests for F28a (#6338): desk queue board shows the kiosk K-number beside its ticket when kioskCode is present.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::DateTime;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

fn queue_of(id: &str) -> String {
    format!("K-{}", id[..8].to_ascii_uppercase())
}

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CALLER_TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";

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
        let schema = xc_testdb::unique_schema_name("f28_kiosk_code");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
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
             VALUES ($1::uuid, $2::uuid, 'Caller Teller', 'ACTIVE', 'test')",
        )
        .bind(CALLER_TELLER_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert caller user");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1::uuid, $2, 'OPEN', now(), $3::uuid, $4::uuid, $3::uuid)",
        )
        .bind(Uuid::new_v4())
        .bind(B01)
        .bind(CALLER_TELLER_ID)
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .expect("insert shift");

        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(ORG)
        .bind(B01)
        .bind(CALLER_TELLER_ID)
        .execute(&pool)
        .await
        .expect("insert counter");

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

        Self { pool, schema }
    }

    async fn finish(self) {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(
                PgConnectOptions::from_str(
                    &std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL"),
                )
                .expect("parse URL"),
            )
            .await
            .expect("admin connect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop test schema");
        self.pool.close().await;
        admin.close().await;
    }
}

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role_part, target_part) = spec.split_once('@')?;
    let role = match role_part {
        "TELLER" => Role::Teller,
        "BRANCH_MANAGER" => Role::BranchManager,
        other => panic!("unknown role {other}"),
    };
    let uid = CALLER_TELLER_ID.to_string();
    let branch = (target_part != "*").then(|| BranchId::new(target_part));
    Some(User::new(
        &uid,
        ORG,
        vec![ScopedRole::new(role, branch, DateTime::UNIX_EPOCH, None)],
        None,
    ))
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok().map(|s| s.to_owned()));
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

struct Answer {
    status: StatusCode,
    body: Value,
}

async fn call(db: &TestDb, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "TELLER@XC-B01"));

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, body }
}

async fn blueprint_booking(pool: &PgPool, branch: &str, id: &str) {
    for sql in [
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-' || $2, organization_id, 'CU-' || $2, 'Booked', 'LIVE', now() FROM branches WHERE id = $1",
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, \
             reserved, status) \
         SELECT gen_random_uuid(), organization_id, id, current_date, now() + interval '1 hour', \
             now() + interval '90 minutes', 4, 1, 'OPEN' FROM branches WHERE id = $1 AND $2 IS NOT NULL",
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
             requested_currency, requested_quantity, source_channel, status, qr_token_hash, created_by) \
         SELECT $2, s.organization_id, s.branch_id, 'B-' || $2, 'CU-' || $2, s.id, 'BUY', 'USD', 100, 'DESK', \
             'CONFIRMED', convert_to('qr-' || $2, 'UTF8'), 'test' \
         FROM booking_slots s WHERE s.branch_id = $1 ORDER BY s.created_at DESC, s.id LIMIT 1",
    ] {
        sqlx::query(sql)
            .bind(branch)
            .bind(id)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("seed booking {id}: {e}"));
    }
}

#[actix_web::test]
async fn test_f28_queue_board_kiosk_code() {
    let db = TestDb::start().await;

    let req_waiting_id = "6cee4945-8fe0-4963-b1dd-c1d013093222";
    let req_called_id = "7dee4945-8fe0-4963-b1dd-c1d013093222";

    for req_id in [req_waiting_id, req_called_id] {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, foreign_amount, thb, input_amount, payment_method) \
             VALUES ($1, $2, 'WAITING', now(), 'BUY', 'USD', 35.0000, 100.00, 3500.00, 100.00, 'Cash')",
        )
        .bind(B01)
        .bind(req_id)
        .execute(&db.pool)
        .await
        .expect("insert kiosk_request");
    }

    let t_kiosk_waiting = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, kiosk_request_id, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now() - interval '5m', $4, 'key-kiosk-waiting')",
    )
    .bind(t_kiosk_waiting)
    .bind(ORG)
    .bind(B01)
    .bind(req_waiting_id)
    .execute(&db.pool)
    .await
    .expect("insert waiting kiosk ticket");

    let t_kiosk_called = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, kiosk_request_id, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 2, 'W002', 'CALLED', $4::uuid, now() - interval '4m', now() - interval '1m', $5, 'key-kiosk-called')",
    )
    .bind(t_kiosk_called)
    .bind(ORG)
    .bind(B01)
    .bind(CALLER_TELLER_ID)
    .bind(req_called_id)
    .execute(&db.pool)
    .await
    .expect("insert called kiosk ticket");

    let t_walk_in = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, kiosk_request_id, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 3, 'W003', 'WAITING', now() - interval '3m', NULL, 'key-walkin')",
    )
    .bind(t_walk_in)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert walk-in ticket");

    let booking_id = "BK-F28";
    blueprint_booking(&db.pool, B01, booking_id).await;

    let t_booking = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, kiosk_request_id, idempotency_key, booking_id) \
         VALUES ($1, $2::uuid, $3, current_date, 'BOOKING', 1, 'B001', 'WAITING', now() - interval '2m', NULL, 'key-booking', $4)",
    )
    .bind(t_booking)
    .bind(ORG)
    .bind(B01)
    .bind(booking_id)
    .execute(&db.pool)
    .await
    .expect("insert booking ticket");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");
    let ans = call(&db, &uri).await;
    assert_eq!(ans.status, StatusCode::OK);

    let waiting = ans.body["data"]["waiting"]
        .as_array()
        .expect("waiting array");
    let called = ans.body["data"]["called"].as_array().expect("called array");

    let w_kiosk = waiting
        .iter()
        .find(|t| t["displayNumber"] == "W001")
        .expect("W001 in waiting");
    assert_eq!(
        w_kiosk["kioskCode"],
        queue_of(req_waiting_id),
        "waiting kiosk ticket has kioskCode matching queue_of"
    );

    let c_kiosk = called
        .iter()
        .find(|t| t["displayNumber"] == "W002")
        .expect("W002 in called");
    assert_eq!(
        c_kiosk["kioskCode"],
        queue_of(req_called_id),
        "called kiosk ticket has kioskCode matching queue_of"
    );

    let w_walkin = waiting
        .iter()
        .find(|t| t["displayNumber"] == "W003")
        .expect("W003 in waiting");
    assert!(
        w_walkin["kioskCode"].is_null(),
        "walk-in ticket has kioskCode null"
    );

    let w_booking = waiting
        .iter()
        .find(|t| t["displayNumber"] == "B001")
        .expect("B001 in waiting");
    assert!(
        w_booking["kioskCode"].is_null(),
        "booking ticket has kioskCode null"
    );

    db.finish().await;
}
