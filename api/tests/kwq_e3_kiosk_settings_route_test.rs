//! KWQ-E3 — `POST /api/kiosk` `settings` (public) and `configureSettings` (`RatesDraft`) through
//! the route table on a real PostgreSQL (design `docs/design/blueprint/kiosk-shared-phone.md`
//! §4.2 D14, §4.4 B18; migration `kiosk_settings`).
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `[required]` wire:
//! - `settings {branchId}` ⇒ `200 {"resultSeconds", "version"}`, defaults `{10, 0}` with no row;
//!   anonymous allowed (the queue machine polls it).
//! - `configureSettings {branchId, resultSeconds}` ⇒ `200 {"resultSeconds", "version"}`, the row
//!   saved with `version` + 1. Out of range (outside 3..=120), not an integer or missing ⇒
//!   `400 {"error":"validationFailed","field":"resultSeconds"}` and nothing saved.
//! - `configureSettings` without `RatesDraft` ⇒ exactly the status and body `configure` answers
//!   the same caller.
//!
//! The harness is `kwq_a5_issue_by_token_route_test.rs`'s (from `kiosk_cancel_route_test.rs`),
//! plus `RATE_MANAGER` (a `RatesDraft` holder).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod common;

#[path = "rates_re4c_common/mod.rs"]
mod rates_re4c_common;

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
const TELLER_A_ID: &str = "00000000-0000-0000-0000-000000000010";
const TELLER_B_ID: &str = "00000000-0000-0000-0000-000000000020";
const MANAGER_ID: &str = "00000000-0000-0000-0000-000000000030";

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
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("kwqe3");
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
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES \
             ('shift-a', $1, 'OPEN', now(), 'test', 'req-open-a', $2::uuid), \
             ('shift-b', $1, 'OPEN', now(), 'test', 'req-open-b', $3::uuid), \
             ('shift-m', $1, 'OPEN', now(), 'test', 'req-open-m', $4::uuid)",
        )
        .bind(B01)
        .bind(TELLER_A_ID)
        .bind(TELLER_B_ID)
        .bind(MANAGER_ID)
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
        "RATE_MANAGER" => (MANAGER_ID, Role::RateManager),
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

const KIOSK_IP: &str = "10.20.30.40";

async fn call(db: &TestDb, who: Option<&str>, ip: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(Method::POST)
        .uri("/api/kiosk")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-real-ip", ip))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
    if let Some(who) = who {
        req = req.insert_header((AS, who));
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn kiosk(db: &TestDb, action: &str, payload: Value) -> Answer {
    call(
        db,
        None,
        KIOSK_IP,
        json!({ "action": action, "payload": payload }),
    )
    .await
}

async fn staff(db: &TestDb, who: &str, action: &str, payload: Value) -> Answer {
    call(
        db,
        Some(&format!("{who}@{B01}")),
        KIOSK_IP,
        json!({ "action": action, "payload": payload }),
    )
    .await
}

/// A quote from the kiosk at `KIOSK_IP`, aged past the 3 s review. Its id.
async fn quoted(db: &TestDb) -> String {
    let ans = kiosk(
        db,
        "quote",
        json!({ "type": "BUY", "currency": "USD", "amount": 100, "branchId": B01 }),
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "quote: {}", ans.text);
    let id = ans.body["id"].as_str().expect("quote id").to_owned();
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '10s' WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("age the quote");
    id
}

/// A confirmed (`WAITING`) request with its walk-in ticket: (id, cancel token).
async fn waiting(db: &TestDb) -> (String, String) {
    let id = quoted(db).await;
    let ans = kiosk(db, "confirm", json!({ "id": id, "branchId": B01 })).await;
    assert_eq!(ans.status, StatusCode::OK, "confirm: {}", ans.text);
    let token = ans.body["cancelToken"]
        .as_str()
        .unwrap_or_else(|| panic!("the first confirm answers a cancelToken: {}", ans.text))
        .to_owned();
    (id, token)
}

/// `status | staff | cancelled_by | cancel_reason | cancel_note` of the request.
async fn request_row(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar(
        "SELECT concat_ws(' | ', status, coalesce(staff, '-'), coalesce(cancelled_by, '-'), \
                coalesce(cancel_reason, '-'), coalesce(cancel_note, '-')) \
         FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
    )
    .bind(B01)
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("read the request")
}

/// `status ended_reason` of the request's ticket.
async fn ticket_row(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar(
        "SELECT status || ' ' || coalesce(ended_reason, '-') FROM queue_tickets \
         WHERE branch_id = $1 AND kiosk_request_id = $2",
    )
    .bind(B01)
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("read the ticket")
}

/// The request's `KIOSK_CANCELLED` lines: (actor, detail).
async fn cancel_lines(db: &TestDb, id: &str) -> Vec<(String, Value)> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor, detail::text FROM device_events \
         WHERE branch_id = $1 AND kind = 'KIOSK_CANCELLED' AND ref = $2 ORDER BY at",
    )
    .bind(B01)
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("read the audit lines");
    rows.into_iter()
        .map(|(actor, detail)| {
            (
                actor,
                serde_json::from_str(&detail).expect("detail is JSON"),
            )
        })
        .collect()
}

fn cancel_payload(id: &str, reason: &str, note: Option<&str>) -> Value {
    let mut payload = json!({ "id": id, "branchId": B01, "reason": reason });
    if let Some(note) = note {
        payload["note"] = json!(note);
    }
    payload
}

fn refused(ans: &Answer, sentence: &str) {
    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{}", ans.text);
    assert_eq!(ans.body, json!({ "error": sentence }), "{}", ans.text);
}

const KIOSK_REFUSED: &str = "ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน";

// ------------------------------------------------------------ the queue machine's settings

fn settings_of(ans: &Answer) -> (i64, i64) {
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        ans.body.as_object().map(|o| o.len()),
        Some(2),
        "only resultSeconds and version: {}",
        ans.text
    );
    (
        ans.body["resultSeconds"].as_i64().expect("resultSeconds"),
        ans.body["version"].as_i64().expect("version"),
    )
}

async fn rows(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_settings")
        .fetch_one(&db.pool)
        .await
        .expect("count")
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_with_no_row_are_the_defaults_and_anyone_may_read_them() {
    let db = TestDb::start().await;
    let ans = kiosk(&db, "settings", json!({ "branchId": B01 })).await;
    assert_eq!(settings_of(&ans), (10, 0));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rates_draft_holder_saves_and_the_version_moves() {
    let db = TestDb::start().await;
    let configure = staff(&db, "RATE_MANAGER", "configure", json!({ "branchId": B01 })).await;
    assert_ne!(configure.status, StatusCode::FORBIDDEN, "precondition: RATE_MANAGER holds RatesDraft: {}", configure.text);

    let saved = staff(&db, "RATE_MANAGER", "configureSettings", json!({ "branchId": B01, "resultSeconds": 25 })).await;
    assert_eq!(settings_of(&saved), (25, 1));
    let read = kiosk(&db, "settings", json!({ "branchId": B01 })).await;
    assert_eq!(settings_of(&read), (25, 1));

    let again = staff(&db, "RATE_MANAGER", "configureSettings", json!({ "branchId": B01, "resultSeconds": 30 })).await;
    assert_eq!(settings_of(&again), (30, 2));
    assert_eq!(rows(&db).await, 1, "one row per branch");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_out_of_range_or_missing_value_is_refused_naming_the_field() {
    let db = TestDb::start().await;
    for bad in [json!(2), json!(121), json!("ten"), json!(null), json!(12.5)] {
        let ans = staff(&db, "RATE_MANAGER", "configureSettings", json!({ "branchId": B01, "resultSeconds": bad })).await;
        assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{bad}: {}", ans.text);
        assert_eq!(ans.body, json!({ "error": "validationFailed", "field": "resultSeconds" }), "{bad}");
    }
    let missing = staff(&db, "RATE_MANAGER", "configureSettings", json!({ "branchId": B01 })).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST, "{}", missing.text);
    assert_eq!(rows(&db).await, 0, "nothing saved");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn without_rates_draft_the_answer_is_configures() {
    let db = TestDb::start().await;
    for who in [None, Some("TELLER_A")] {
        let configure = match who {
            None => kiosk(&db, "configure", json!({ "branchId": B01 })).await,
            Some(w) => staff(&db, w, "configure", json!({ "branchId": B01 })).await,
        };
        let ans = match who {
            None => kiosk(&db, "configureSettings", json!({ "branchId": B01, "resultSeconds": 20 })).await,
            Some(w) => staff(&db, w, "configureSettings", json!({ "branchId": B01, "resultSeconds": 20 })).await,
        };
        assert_eq!((ans.status, &ans.body), (configure.status, &configure.body), "{who:?}: {}", ans.text);
    }
    assert_eq!(rows(&db).await, 0);
    db.finish().await;
}
