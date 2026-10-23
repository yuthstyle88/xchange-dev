//! KWQ-A5 — `POST /api/kiosk` `issueByToken` and `findByToken`, end to end through the route
//! table on a real PostgreSQL (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D5, D7,
//! D9; §4.3a W8; §4.4 B6–B10, B13): the staff scan at the queue machine issues the web request's
//! ticket with the member on it and answers `{displayNumber, ticketId, ticketEscpos}`; a repeat
//! answers the same; every token that leads nowhere answers `404 {"error":"notFound"}`; without
//! `QueueOperate` the answer is the policy's, as for `accept`.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `[required]` wire shapes: `issueByToken {branchId, token}` → `200 {displayNumber, ticketId,
//! ticketEscpos}` (`ticketEscpos` = standard base64 of `TicketPrintJob::render`, whose QR is
//! `<Origin header>/q#<token>`); `findByToken {branchId, token}` → `200 {requestId}`; a token that
//! leads nowhere → `404 {"error":"notFound"}` from both.
//!
//! The harness (database, three staff, header sessions) is copied from
//! `kiosk_cancel_route_test.rs`. A web request is made here as `webRequestCreate` will make it
//! (slice c): a kiosk `quote`, then its token hash and member set in SQL.
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
        let schema = unique("kwqa5");
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

// ------------------------------------------------------------ the web request's scan

use base64::Engine as _;

/// A QUOTED web request of `B01` whose request token is `token` and whose member is `cust-1`.
async fn web_request(db: &TestDb, token: &str) -> String {
    let id = quoted(db).await;
    sqlx::query(
        "UPDATE kiosk_requests SET cancel_token_hash = encode(sha256(convert_to($2, 'UTF8')), 'hex'), \
         customer_id = 'cust-1' WHERE id = $1",
    )
    .bind(&id)
    .bind(token)
    .execute(&db.pool)
    .await
    .expect("make it a web request");
    id
}

fn scan(token: &str) -> Value {
    json!({ "branchId": B01, "token": token })
}

fn not_found(ans: &Answer) {
    assert_eq!(ans.status, StatusCode::NOT_FOUND, "{}", ans.text);
    assert_eq!(ans.body, json!({ "error": "notFound" }), "{}", ans.text);
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_staff_scan_issues_the_ticket_with_the_member_and_answers_the_printed_ticket() {
    let db = TestDb::start().await;
    let id = web_request(&db, "tok-1").await;

    let ans = staff(&db, "TELLER_A", "issueByToken", scan("tok-1")).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    let number = ans.body["displayNumber"]
        .as_str()
        .expect("displayNumber")
        .to_owned();
    let ticket_id = ans.body["ticketId"].as_str().expect("ticketId").to_owned();
    let escpos = base64::engine::general_purpose::STANDARD
        .decode(ans.body["ticketEscpos"].as_str().expect("ticketEscpos"))
        .expect("standard base64");
    assert!(
        contains(&escpos, number.as_bytes()),
        "the number is printed"
    );
    assert!(
        contains(&escpos, format!("{SAME_ORIGIN}/q#tok-1").as_bytes()),
        "the request QR is printed"
    );
    assert_eq!(
        ans.body.as_object().map(|o| o.len()),
        Some(3),
        "nothing else is answered: {}",
        ans.text
    );

    let row: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT t.display_number, t.customer_id, r.status FROM queue_tickets t \
         JOIN kiosk_requests r ON r.id = t.kiosk_request_id \
         WHERE t.id = $1::uuid AND r.id = $2",
    )
    .bind(&ticket_id)
    .bind(&id)
    .fetch_one(&db.pool)
    .await
    .expect("the ticket of the request");
    assert_eq!(
        row,
        (
            number.clone(),
            Some("cust-1".to_owned()),
            Some("WAITING".to_owned())
        )
    );

    // a repeat scan: the same ticket, the same bytes, no second number
    let again = staff(&db, "TELLER_B", "issueByToken", scan("tok-1")).await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(again.body, ans.body);
    let tickets: i64 =
        sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE kiosk_request_id = $1")
            .bind(&id)
            .fetch_one(&db.pool)
            .await
            .expect("count");
    assert_eq!(tickets, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_desk_lookup_answers_the_request_id() {
    let db = TestDb::start().await;
    let id = web_request(&db, "tok-1").await;
    let ans = staff(&db, "BRANCH_MANAGER", "findByToken", scan("tok-1")).await;
    assert_eq!(ans.status, StatusCode::OK, "{}", ans.text);
    assert_eq!(ans.body, json!({ "requestId": id }));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_that_leads_nowhere_answers_not_found_and_issues_nothing() {
    let db = TestDb::start().await;
    let id = web_request(&db, "tok-1").await;

    not_found(&staff(&db, "TELLER_A", "issueByToken", scan("nope")).await);
    not_found(&staff(&db, "TELLER_A", "findByToken", scan("nope")).await);
    not_found(&staff(&db, "TELLER_A", "issueByToken", scan("")).await);

    // expired: past the purge window
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '2 hours' WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("age it");
    not_found(&staff(&db, "TELLER_A", "issueByToken", scan("tok-1")).await);
    not_found(&staff(&db, "TELLER_A", "findByToken", scan("tok-1")).await);

    let tickets: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_tickets")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(tickets, 0);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn without_queue_operate_the_answer_is_the_policys_as_for_accept() {
    let db = TestDb::start().await;
    let id = web_request(&db, "tok-1").await;

    let accept = kiosk(&db, "accept", json!({ "id": id, "branchId": B01 })).await;
    for action in ["issueByToken", "findByToken"] {
        let ans = kiosk(&db, action, scan("tok-1")).await;
        assert_eq!(
            (ans.status, &ans.body),
            (accept.status, &accept.body),
            "{action} without a staff session answers as accept does: {}",
            ans.text
        );
    }
    let tickets: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_tickets")
        .fetch_one(&db.pool)
        .await
        .expect("count");
    assert_eq!(tickets, 0);
    db.finish().await;
}
