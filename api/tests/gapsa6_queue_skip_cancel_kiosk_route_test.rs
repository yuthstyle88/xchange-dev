//! XC-BE-GAPS-A row 6, end to end: a staff skip or cancel of a kiosk-linked queue ticket over
//! `/api/queue/tickets/{id}/skip` and `/api/queue/tickets/{id}/cancel` also closes the kiosk request it came
//! from, on a real PostgreSQL through the route table.
//!
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 6, 2026-10-27).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md:207` (§3.7: "A staff `ticketCancel` / skip on a kiosk-linked
//! ticket should also cancel the request. Today it leaves it stuck"), §1 last bullet (the stuck row), §2's table
//! `:27-35` (skip is `NO_SHOW` ⇔ `CUSTOMER_LEFT`), the reason list `:36-48`, §3.3 (the cancel path).
//!
//! Pinned here:
//! - skip of a CALLED kiosk-linked ticket: the ticket `NO_SHOW` as today, with one `SKIP` event and no second
//!   move; the request `CANCELLED` `CUSTOMER_LEFT`, `cancelled_by` the staff id; one `KIOSK_CANCELLED` line
//!   naming the staff;
//! - cancel with `DUPLICATE`: the request `CANCELLED` `DUPLICATE`; with free text: `OTHER` with the (trimmed)
//!   text as `cancel_note`; the ticket keeps the queue's own `ended_reason` (as today), one `CANCEL` event;
//! - the request's `cancelled_by` is whoever ended the ticket, not whoever called it;
//! - a request already `COMPLETED` or `CANCELLED`: the queue action answers 200, the request is unchanged and
//!   no kiosk line is written; a ticket with no kiosk request: as today.
//!
//! The harness (database, two tellers and a manager on duty, header sessions, the kiosk's quote → confirm) is
//! copied from `kiosk_cancel_route_test.rs`; `queue` posts a queue command as `queue_commands_views_test.rs`
//! does (body `{ "branch": … }`, an `Idempotency-Key`).
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
        let schema = unique("gapsa6");
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
        xc_testdb::seed_branch_currencies(&pool, B01, &["USD"])
            .await
            .expect("seed branch currencies");
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

async fn call(db: &TestDb, who: Option<&str>, uri: &str, body: Value, key: Option<&str>) -> Answer {
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
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-real-ip", KIOSK_IP))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
    if let Some(who) = who {
        req = req.insert_header((AS, who));
    }
    if let Some(key) = key {
        req = req.insert_header(("idempotency-key", key));
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
        "/api/kiosk",
        json!({ "action": action, "payload": payload }),
        None,
    )
    .await
}

async fn staff(db: &TestDb, who: &str, action: &str, payload: Value) -> Answer {
    call(
        db,
        Some(&format!("{who}@{B01}")),
        "/api/kiosk",
        json!({ "action": action, "payload": payload }),
        None,
    )
    .await
}

/// A queue command `/api/queue/tickets/{ticket}/{command}` by `who`, with `extra` beside `branch`.
async fn queue(db: &TestDb, who: &str, ticket: &str, command: &str, extra: Value) -> Answer {
    let mut body = json!({ "branch": B01 });
    if let Value::Object(more) = extra {
        for (k, v) in more {
            body[k] = v;
        }
    }
    call(
        db,
        Some(&format!("{who}@{B01}")),
        &format!("/api/queue/tickets/{ticket}/{command}"),
        body,
        Some(&unique("k-gapsa6")),
    )
    .await
}

fn ok(ans: &Answer, what: &str) {
    assert_eq!(ans.status, StatusCode::OK, "{what}: {}", ans.text);
}

/// A confirmed (`WAITING`) request with its walk-in ticket: (request id, ticket id).
async fn waiting(db: &TestDb) -> (String, String) {
    let ans = kiosk(
        db,
        "quote",
        json!({ "type": "BUY", "currency": "USD", "amount": 100, "branchId": B01 }),
    )
    .await;
    ok(&ans, "quote");
    let id = ans.body["id"].as_str().expect("quote id").to_owned();
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '10s' WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("age the quote");
    let ans = staff(
        db,
        "TELLER_A",
        "confirm",
        json!({ "id": id, "branchId": B01 }),
    )
    .await;
    ok(&ans, "confirm");
    let ticket: String = sqlx::query_scalar(
        "SELECT id::text FROM queue_tickets WHERE branch_id = $1 AND kiosk_request_id = $2",
    )
    .bind(B01)
    .bind(&id)
    .fetch_one(&db.pool)
    .await
    .expect("confirm issued the request's ticket");
    (id, ticket)
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

/// `status ended_reason` of a ticket.
async fn ticket_row(db: &TestDb, ticket: &str) -> String {
    sqlx::query_scalar(
        "SELECT status || ' ' || coalesce(ended_reason, '-') FROM queue_tickets WHERE id = $1::uuid",
    )
    .bind(ticket)
    .fetch_one(&db.pool)
    .await
    .expect("read the ticket")
}

/// The kinds of a ticket's events, oldest first.
async fn event_kinds(db: &TestDb, ticket: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT kind FROM queue_events WHERE ticket_id = $1::uuid ORDER BY at, created_at",
    )
    .bind(ticket)
    .fetch_all(&db.pool)
    .await
    .expect("read the events")
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

async fn all_cancel_lines(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM device_events WHERE kind = 'KIOSK_CANCELLED'")
        .fetch_one(&db.pool)
        .await
        .expect("count the audit lines")
}

// ------------------------------------------------------------ skip

#[actix_web::test]
async fn gapsa6_skipping_a_called_kiosk_ticket_cancels_its_request_customer_left() {
    let db = TestDb::start().await;
    let (id, ticket) = waiting(&db).await;
    ok(
        &queue(&db, "TELLER_A", &ticket, "call", json!({})).await,
        "call",
    );

    let ans = queue(&db, "TELLER_A", &ticket, "skip", json!({})).await;

    ok(&ans, "skip");
    assert_eq!(ans.body["status"], "NO_SHOW", "{}", ans.text);
    assert_eq!(
        ticket_row(&db, &ticket).await,
        "NO_SHOW -",
        "the skip as today"
    );
    assert_eq!(
        event_kinds(&db, &ticket).await,
        ["ISSUE", "CALL", "SKIP"],
        "one SKIP event; the kiosk does not move the ticket a second time"
    );
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | - | {TELLER_A_ID} | CUSTOMER_LEFT | -")
    );
    let lines = cancel_lines(&db, &id).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        lines[0].0, "Teller A",
        "the line names the staff member (`PgKioskAuditLog` writes the user's name)"
    );
    assert_eq!(
        lines[0].1,
        json!({ "from": "WAITING", "reason": "CUSTOMER_LEFT", "note": null })
    );
    let status = kiosk(&db, "status", json!({ "id": id, "branchId": B01 })).await;
    assert_eq!(status.body["status"], "CANCELLED", "{}", status.text);
    db.finish().await;
}

// ------------------------------------------------------------ cancel

#[actix_web::test]
async fn gapsa6_cancelling_a_kiosk_ticket_duplicate_cancels_its_request_duplicate() {
    let db = TestDb::start().await;
    let (id, ticket) = waiting(&db).await;

    let ans = queue(
        &db,
        "TELLER_A",
        &ticket,
        "cancel",
        json!({ "reason": "DUPLICATE" }),
    )
    .await;

    ok(&ans, "cancel");
    assert_eq!(
        ticket_row(&db, &ticket).await,
        "CANCELLED DUPLICATE",
        "the queue's own ended_reason, as today"
    );
    assert_eq!(event_kinds(&db, &ticket).await, ["ISSUE", "CANCEL"]);
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | - | {TELLER_A_ID} | DUPLICATE | -")
    );
    let lines = cancel_lines(&db, &id).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, "Teller A");
    assert_eq!(
        lines[0].1,
        json!({ "from": "WAITING", "reason": "DUPLICATE", "note": null })
    );
    db.finish().await;
}

/// Teller A calls the ticket, teller B cancels it: the request records B, the one who ended it.
#[actix_web::test]
async fn gapsa6_cancelling_with_free_text_cancels_its_request_other_with_the_text() {
    let db = TestDb::start().await;
    let (id, ticket) = waiting(&db).await;
    ok(
        &queue(&db, "TELLER_A", &ticket, "call", json!({})).await,
        "call",
    );

    let ans = queue(
        &db,
        "TELLER_B",
        &ticket,
        "cancel",
        json!({ "reason": "  ลูกค้าแจ้งว่าจะมาใหม่พรุ่งนี้  " }),
    )
    .await;

    ok(&ans, "cancel");
    assert_eq!(
        ticket_row(&db, &ticket).await,
        "CANCELLED ลูกค้าแจ้งว่าจะมาใหม่พรุ่งนี้"
    );
    assert_eq!(event_kinds(&db, &ticket).await, ["ISSUE", "CALL", "CANCEL"]);
    assert_eq!(
        request_row(&db, &id).await,
        format!("CANCELLED | - | {TELLER_B_ID} | OTHER | ลูกค้าแจ้งว่าจะมาใหม่พรุ่งนี้")
    );
    let lines = cancel_lines(&db, &id).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, "Teller B");
    assert_eq!(
        lines[0].1,
        json!({ "from": "WAITING", "reason": "OTHER", "note": "ลูกค้าแจ้งว่าจะมาใหม่พรุ่งนี้" })
    );
    db.finish().await;
}

// ------------------------------------------------------------ nothing to close

/// A request already `COMPLETED` or `CANCELLED`, and a ticket with no request: the queue action answers 200
/// and changes the ticket as today, and the kiosk writes nothing. A live request beside them is closed, so a
/// route that never reaches the kiosk does not pass.
#[actix_web::test]
async fn gapsa6_an_ended_request_or_a_plain_ticket_is_left_alone() {
    let db = TestDb::start().await;
    let (done, done_ticket) = waiting(&db).await;
    let (gone, gone_ticket) = waiting(&db).await;
    let (live, live_ticket) = waiting(&db).await;
    sqlx::query(
        "UPDATE kiosk_requests SET status = 'COMPLETED', staff = $2 WHERE branch_id = $1 AND id = $3",
    )
    .bind(B01)
    .bind(TELLER_A_ID)
    .bind(&done)
    .execute(&db.pool)
    .await
    .expect("a completed request whose ticket still waits");
    sqlx::query(
        "UPDATE kiosk_requests SET status = 'CANCELLED', cancelled_at = now(), cancelled_by = 'earlier', \
         cancel_reason = 'CUSTOMER_CHANGED_MIND' WHERE branch_id = $1 AND id = $2",
    )
    .bind(B01)
    .bind(&gone)
    .execute(&db.pool)
    .await
    .expect("a cancelled request whose ticket still waits");
    let plain = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
             display_number, status, issued_at, idempotency_key) \
         VALUES ($1::uuid, $2::uuid, $3, current_date, 'WALK_IN', 900, 'W900', 'WAITING', now(), 'key-gapsa6-plain')",
    )
    .bind(&plain)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("a desk walk-in with no kiosk request");
    let done_before = request_row(&db, &done).await;
    let gone_before = request_row(&db, &gone).await;

    let ans = queue(
        &db,
        "TELLER_A",
        &done_ticket,
        "cancel",
        json!({ "reason": "DUPLICATE" }),
    )
    .await;
    ok(&ans, "cancel the completed request's ticket");
    ok(
        &queue(&db, "TELLER_A", &gone_ticket, "call", json!({})).await,
        "call",
    );
    ok(
        &queue(&db, "TELLER_A", &gone_ticket, "skip", json!({})).await,
        "skip the cancelled request's ticket",
    );
    ok(
        &queue(
            &db,
            "TELLER_A",
            &plain,
            "cancel",
            json!({ "reason": "CUSTOMER_LEFT" }),
        )
        .await,
        "cancel the plain ticket",
    );

    assert_eq!(ticket_row(&db, &done_ticket).await, "CANCELLED DUPLICATE");
    assert_eq!(ticket_row(&db, &gone_ticket).await, "NO_SHOW -");
    assert_eq!(ticket_row(&db, &plain).await, "CANCELLED CUSTOMER_LEFT");
    assert_eq!(request_row(&db, &done).await, done_before);
    assert_eq!(request_row(&db, &gone).await, gone_before);
    assert_eq!(all_cancel_lines(&db).await, 0, "the kiosk wrote no line");

    ok(
        &queue(
            &db,
            "TELLER_A",
            &live_ticket,
            "cancel",
            json!({ "reason": "CUSTOMER_LEFT" }),
        )
        .await,
        "cancel the live request's ticket",
    );
    assert_eq!(
        ticket_row(&db, &live_ticket).await,
        "CANCELLED CUSTOMER_LEFT"
    );
    assert_eq!(
        request_row(&db, &live).await,
        format!("CANCELLED | - | {TELLER_A_ID} | CUSTOMER_LEFT | -")
    );
    assert_eq!(all_cancel_lines(&db).await, 1);
    db.finish().await;
}
