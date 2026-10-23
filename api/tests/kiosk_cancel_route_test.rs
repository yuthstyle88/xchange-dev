//! KC-S — `POST /api/kiosk` `cancel` (staff, reason required) and `customerCancel` (the kiosk,
//! with the cancel token from `confirm`), end to end through the route table on a real
//! PostgreSQL: the request row, its walk-in ticket, the `KIOSK_CANCELLED` audit line, and the
//! answers.
//! Written before the implementation by the ticket author (claude/studio-claude-10). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md` §2, §3.3 (`cancel_with`), §3.4.
//! The harness (database, three staff, header sessions) is copied from
//! `kiosk_accept_serving_other_teller_test.rs`; `call` here also sends the kiosk's address
//! (`x-real-ip`, `client_address.rs:20`).
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
use uuid::Uuid;
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
        let schema = unique("kcs");
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
    let ans = staff(db, "TELLER_A", "confirm", json!({ "id": id, "branchId": B01 })).await;
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

// ------------------------------------------------------------ confirm's token

#[actix_web::test]
async fn the_first_confirm_answers_a_cancel_token_and_stores_only_its_hash() {
    let db = TestDb::start().await;
    let id = quoted(&db).await;

    let first = staff(&db, "TELLER_A", "confirm", json!({ "id": id, "branchId": B01 })).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let token = first.body["cancelToken"].as_str().expect("cancelToken");
    assert!(
        Uuid::parse_str(token).is_ok_and(|u| u.get_version_num() == 4),
        "a uuid v4: {token}"
    );
    assert_eq!(first.body["status"], "WAITING");

    let stored: Option<String> =
        sqlx::query_scalar("SELECT cancel_token_hash FROM kiosk_requests WHERE id = $1")
            .bind(&id)
            .fetch_one(&db.pool)
            .await
            .expect("read the hash");
    let expected: String =
        sqlx::query_scalar("SELECT encode(sha256(convert_to($1, 'UTF8')), 'hex')")
            .bind(token)
            .fetch_one(&db.pool)
            .await
            .expect("hash");
    assert_eq!(stored, Some(expected));

    let again = staff(&db, "TELLER_A", "confirm", json!({ "id": id, "branchId": B01 })).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert!(
        again.body.get("cancelToken").is_none(),
        "a repeated confirm has no cancelToken key: {}",
        again.text
    );
    let status = kiosk(&db, "status", json!({ "id": id, "branchId": B01 })).await;
    assert!(status.body.get("cancelToken").is_none(), "{}", status.text);
    db.finish().await;
}

// ------------------------------------------------------------ staff cancel

#[actix_web::test]
async fn a_teller_cancels_a_waiting_request_the_customer_left_and_its_ticket_is_no_show() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;

    let ans = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "CUSTOMER_LEFT", None),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["id"], json!(id));
    assert_eq!(ans.body["status"], "CANCELLED");
    for key in ["queue", "orderNo"] {
        assert!(ans.body[key].is_string(), "{key}: {}", ans.text);
    }
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | - | {TELLER_A_ID} | CUSTOMER_LEFT | -")
    );
    assert_eq!(
        ticket_row(&db, &id).await,
        "NO_SHOW kioskCancelled:CUSTOMER_LEFT"
    );
    let lines = cancel_lines(&db, &id).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        lines[0].1,
        json!({ "from": "WAITING", "reason": "CUSTOMER_LEFT", "note": null })
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_cancel_for_another_reason_cancels_the_ticket_and_keeps_the_trimmed_note() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;

    let ans = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "OTHER", Some("  ลูกค้าโทรมายกเลิก  ")),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | - | {TELLER_A_ID} | OTHER | ลูกค้าโทรมายกเลิก")
    );
    assert_eq!(ticket_row(&db, &id).await, "CANCELLED kioskCancelled:OTHER");
    assert_eq!(
        cancel_lines(&db, &id).await[0].1,
        json!({ "from": "WAITING", "reason": "OTHER", "note": "ลูกค้าโทรมายกเลิก" })
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_bad_reason_is_refused_naming_the_field_and_nothing_changes() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;
    let before = request_row(&db, &id).await;

    let missing = staff(
        &db,
        "TELLER_A",
        "cancel",
        json!({ "id": id, "branchId": B01 }),
    )
    .await;
    refused(&missing, "กรุณาเลือกเหตุผลการยกเลิก (reason)");
    let empty = staff(&db, "TELLER_A", "cancel", cancel_payload(&id, "", None)).await;
    refused(&empty, "กรุณาเลือกเหตุผลการยกเลิก (reason)");
    for code in ["CUSTOMER_CANCELLED", "DAY_CLOSED", "LEFT"] {
        let ans = staff(&db, "TELLER_A", "cancel", cancel_payload(&id, code, None)).await;
        refused(&ans, "เหตุผลการยกเลิกไม่ถูกต้อง (reason)");
    }
    let no_note = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "OTHER", Some("  ")),
    )
    .await;
    refused(&no_note, "กรุณาระบุรายละเอียดเหตุผล (note)");
    let long = "ก".repeat(201);
    let too_long = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "DUPLICATE", Some(&long)),
    )
    .await;
    refused(&too_long, "รายละเอียดเหตุผลต้องไม่เกิน 200 ตัวอักษร (note)");

    assert_eq!(request_row(&db, &id).await, before);
    assert_eq!(ticket_row(&db, &id).await, "WAITING -");
    assert!(cancel_lines(&db, &id).await.is_empty());
    db.finish().await;
}

#[actix_web::test]
async fn a_repeated_cancel_answers_the_same_and_writes_nothing_more() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;
    let first = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "DUPLICATE", None),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    let again = staff(
        &db,
        "TELLER_B",
        "cancel",
        cancel_payload(&id, "CUSTOMER_LEFT", None),
    )
    .await;

    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.body, first.body);
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | - | {TELLER_A_ID} | DUPLICATE | -"),
        "the first cancel's record stands"
    );
    assert_eq!(cancel_lines(&db, &id).await.len(), 1);
    db.finish().await;
}

#[actix_web::test]
async fn the_holder_cancels_an_accepted_request_and_the_serving_ticket_is_cancelled() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;
    let accept = staff(
        &db,
        "TELLER_A",
        "accept",
        json!({ "id": id, "branchId": B01 }),
    )
    .await;
    assert_eq!(accept.status, StatusCode::OK, "accept: {}", accept.text);
    assert!(ticket_row(&db, &id).await.starts_with("SERVING"));

    let ans = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "CUSTOMER_LEFT", None),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | {TELLER_A_ID} | {TELLER_A_ID} | CUSTOMER_LEFT | -"),
        "staff keeps the holder"
    );
    assert_eq!(
        ticket_row(&db, &id).await,
        "CANCELLED kioskCancelled:CUSTOMER_LEFT",
        "a served ticket is never NO_SHOW"
    );
    assert_eq!(cancel_lines(&db, &id).await[0].1["from"], json!("ACCEPTED"));
    db.finish().await;
}

#[actix_web::test]
async fn another_teller_is_refused_and_a_branch_manager_may_cancel_a_held_request() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;
    let accept = staff(
        &db,
        "TELLER_A",
        "accept",
        json!({ "id": id, "branchId": B01 }),
    )
    .await;
    assert_eq!(accept.status, StatusCode::OK, "accept: {}", accept.text);

    let other = staff(
        &db,
        "TELLER_B",
        "cancel",
        cancel_payload(&id, "DUPLICATE", None),
    )
    .await;
    refused(&other, "คำขอนี้มีพนักงานคนอื่นรับเรื่องอยู่");
    assert!(request_row(&db, &id).await.starts_with("ACCEPTED"));
    assert!(ticket_row(&db, &id).await.starts_with("SERVING"));

    let manager = staff(
        &db,
        "BRANCH_MANAGER",
        "cancel",
        cancel_payload(&id, "DUPLICATE", None),
    )
    .await;
    assert_eq!(manager.status, StatusCode::OK, "{}", manager.text);
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | {TELLER_A_ID} | {MANAGER_ID} | DUPLICATE | -")
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_quote_nobody_confirmed_is_not_cancellable() {
    let db = TestDb::start().await;
    let id = quoted(&db).await;

    let ans = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "CUSTOMER_LEFT", None),
    )
    .await;

    refused(&ans, "คำขอนี้ยกเลิกไม่ได้");
    assert!(request_row(&db, &id).await.starts_with("QUOTED"));
    db.finish().await;
}

#[actix_web::test]
async fn nobody_signed_in_cannot_use_the_staff_cancel() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;

    let ans = kiosk(&db, "cancel", cancel_payload(&id, "CUSTOMER_LEFT", None)).await;

    assert_ne!(ans.status, StatusCode::OK, "{}", ans.text);
    assert!(request_row(&db, &id).await.starts_with("WAITING"));
    assert_eq!(ticket_row(&db, &id).await, "WAITING -");
    db.finish().await;
}

// ------------------------------------------------------------ after a cancel

#[actix_web::test]
async fn a_cancelled_request_reads_cancelled_cannot_be_accepted_and_the_desk_says_so() {
    let db = TestDb::start().await;
    let (id, _) = waiting(&db).await;
    let cancel = staff(
        &db,
        "TELLER_A",
        "cancel",
        cancel_payload(&id, "CUSTOMER_LEFT", None),
    )
    .await;
    assert_eq!(cancel.status, StatusCode::OK, "{}", cancel.text);

    let status = kiosk(&db, "status", json!({ "id": id, "branchId": B01 })).await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.text);
    assert_eq!(status.body["status"], "CANCELLED");

    let accept = staff(
        &db,
        "TELLER_B",
        "accept",
        json!({ "id": id, "branchId": B01 }),
    )
    .await;
    refused(&accept, "คิวนี้ไม่พร้อมรับเรื่อง กรุณาโหลดล่าสุด");

    let get = staff(&db, "TELLER_A", "get", json!({ "id": id, "branchId": B01 })).await;
    refused(&get, "คำขอนี้ถูกยกเลิกแล้ว");
    db.finish().await;
}

// ------------------------------------------------------------ the customer's cancel

#[actix_web::test]
async fn the_customer_cancels_at_the_kiosk_with_the_token_and_the_ticket_is_cancelled() {
    let db = TestDb::start().await;
    let (id, token) = waiting(&db).await;

    let ans = kiosk(
        &db,
        "customerCancel",
        json!({ "id": id, "branchId": B01, "cancelToken": token }),
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body["status"], "CANCELLED");
    assert_eq!(
        request_row(&db, &id).await,
        "CANCELLED | - | CUSTOMER-KIOSK | CUSTOMER_CANCELLED | -"
    );
    assert_eq!(
        ticket_row(&db, &id).await,
        "CANCELLED kioskCancelled:CUSTOMER_CANCELLED",
        "the customer is present: not NO_SHOW"
    );
    let lines = cancel_lines(&db, &id).await;
    assert_eq!(
        lines,
        [(
            "CUSTOMER-KIOSK".to_owned(),
            json!({ "from": "WAITING", "reason": "CUSTOMER_CANCELLED", "note": null })
        )]
    );
    db.finish().await;
}

#[actix_web::test]
async fn every_refused_customer_cancel_gets_the_same_answer_and_changes_nothing() {
    let db = TestDb::start().await;
    let (id, token) = waiting(&db).await;
    let (other_id, _) = waiting(&db).await;
    let before = request_row(&db, &id).await;

    let cases = [
        ("no token", json!({ "id": id, "branchId": B01 }), KIOSK_IP),
        (
            "empty token",
            json!({ "id": id, "branchId": B01, "cancelToken": "" }),
            KIOSK_IP,
        ),
        (
            "wrong token",
            json!({ "id": id, "branchId": B01, "cancelToken": Uuid::new_v4().to_string() }),
            KIOSK_IP,
        ),
        (
            "another request's id",
            json!({ "id": other_id, "branchId": B01, "cancelToken": token }),
            KIOSK_IP,
        ),
        (
            "unknown id",
            json!({ "id": "no-such-request", "branchId": B01, "cancelToken": token }),
            KIOSK_IP,
        ),
        (
            "another address",
            json!({ "id": id, "branchId": B01, "cancelToken": token }),
            "10.20.30.41",
        ),
    ];
    for (what, payload, ip) in cases {
        let ans = call(
            &db,
            None,
            ip,
            json!({ "action": "customerCancel", "payload": payload }),
        )
        .await;
        assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{what}: {}", ans.text);
        assert_eq!(ans.body, json!({ "error": KIOSK_REFUSED }), "{what}");
    }
    assert_eq!(request_row(&db, &id).await, before);
    assert_eq!(ticket_row(&db, &id).await, "WAITING -");
    assert!(request_row(&db, &other_id).await.starts_with("WAITING"));
    assert!(cancel_lines(&db, &id).await.is_empty());
    db.finish().await;
}

#[actix_web::test]
async fn the_customer_cannot_cancel_once_staff_accepted_it() {
    let db = TestDb::start().await;
    let (id, token) = waiting(&db).await;
    let accept = staff(
        &db,
        "TELLER_A",
        "accept",
        json!({ "id": id, "branchId": B01 }),
    )
    .await;
    assert_eq!(accept.status, StatusCode::OK, "accept: {}", accept.text);

    let ans = kiosk(
        &db,
        "customerCancel",
        json!({ "id": id, "branchId": B01, "cancelToken": token }),
    )
    .await;

    refused(&ans, KIOSK_REFUSED);
    assert!(request_row(&db, &id).await.starts_with("ACCEPTED"));
    assert!(ticket_row(&db, &id).await.starts_with("SERVING"));
    db.finish().await;
}
