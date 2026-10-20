//! S5 of the FE survey section B (#3408; design `docs/design/blueprint/section-b-gaps.md` §S5, PR 3387).
//!
//! 🔴 Written by the ticket writer claude/studio-claude-4 before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own. The harness above the tests is copied from
//! `queue_commands_views_test.rs` (QU-7).
//!
//! `my_queue_ticket?branch=` **without** `ticketId` answers the session customer's newest ticket at
//! that branch with status WAITING, CALLED or SERVING on today's service date, as the same
//! `MyQueueTicketData`; none ⇒ 404 `noTicket`.

#![allow(dead_code)]

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
use serde_json::Value;
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
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
const MANAGER_ID: &str = "00000000-0000-0000-0000-000000000020";
const FINANCE_ID: &str = "00000000-0000-0000-0000-000000000030";
const CUSTOMER_ALICE_ID: &str = "00000000-0000-0000-0000-000000000040";
const CUSTOMER_BOB_ID: &str = "00000000-0000-0000-0000-000000000050";

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
        let schema = unique("qu7test");
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
               ($1::uuid, $6::uuid, 'Teller 1', 'ACTIVE', 'test'), \
               ($2::uuid, $6::uuid, 'Manager 1', 'ACTIVE', 'test'), \
               ($3::uuid, $6::uuid, 'Finance 1', 'ACTIVE', 'test'), \
               ($4::uuid, $6::uuid, 'Customer Alice', 'ACTIVE', 'test'), \
               ($5::uuid, $6::uuid, 'Customer Bob', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID)
        .bind(MANAGER_ID)
        .bind(FINANCE_ID)
        .bind(CUSTOMER_ALICE_ID)
        .bind(CUSTOMER_BOB_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert test users");

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

fn user_id_for_role(role_name: &str) -> String {
    match role_name {
        "TELLER" => TELLER_ID.to_string(),
        "BRANCH_MANAGER" => MANAGER_ID.to_string(),
        "FINANCE" => FINANCE_ID.to_string(),
        "CUSTOMER" => CUSTOMER_ALICE_ID.to_string(),
        _ => Uuid::new_v4().to_string(),
    }
}

fn user_of(spec: &str) -> Option<User> {
    let (role_part, target_part) = spec.split_once('@')?;
    let role = match role_part {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "CUSTOMER" => Role::Customer,
        other => panic!("unknown role {other}"),
    };

    if role == Role::Customer {
        let uid = if target_part == "customer-alice" {
            CUSTOMER_ALICE_ID.to_string()
        } else if target_part == "customer-bob" {
            CUSTOMER_BOB_ID.to_string()
        } else {
            Uuid::new_v4().to_string()
        };
        return Some(User::new(
            &uid,
            "XC",
            vec![ScopedRole::new(
                Role::Customer,
                None,
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
            Some(target_part),
        ));
    }

    let uid = user_id_for_role(role_part);
    let branch = (target_part != "*").then(|| BranchId::new(target_part));
    Some(User::new(
        &uid,
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

// ------------------------------------------------------------ requests

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
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

/// A policy refusal: the role code and a client-error status (`PolicyError::status`; the shape of
/// `tx4_settings_routes_test.rs::policy_refused`).
fn policy_refused(a: &Answer, what: &str) {
    assert!(
        a.status.is_client_error() && a.body["error"]["code"] == "roleNotAllowedHere",
        "{what}: expected a client-error roleNotAllowedHere refusal; got {} {}",
        a.status,
        a.text
    );
}

/// A domain refusal with `code` and a client-error status.
fn refused_with(a: &Answer, code: &str, what: &str) {
    assert!(
        a.status.is_client_error() && a.body["error"]["code"] == code,
        "{what}: expected a client-error {code} refusal; got {} {}",
        a.status,
        a.text
    );
}

async fn ticket_row(db: &TestDb, id: Uuid) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT status, counter_teller_id::text, ended_reason FROM queue_tickets WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("read the ticket")
}

// ------------------------------------------------------------ tests

async fn s5_ticket(db: &TestDb, seq: i32, status: &str, customer: Option<&str>, day: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
           display_number, status, issued_at, idempotency_key, customer_id) \
         VALUES ($1, $2::uuid, $3, {day}, 'WALK_IN', $4, $5, $6, now() - interval '5m', $7, $8)"
    )))
    .bind(id)
    .bind(ORG)
    .bind(B01)
    .bind(seq)
    .bind(format!("W{seq:03}"))
    .bind(status)
    .bind(format!("s5-{id}"))
    .bind(customer)
    .execute(&db.pool)
    .await
    .expect("insert a queue ticket");
    id
}

fn s5_uri() -> String {
    format!("/api/v3/views/my_queue_ticket?branch={B01}")
}

/// B1: a customer holding a WAITING ticket today gets it without naming it.
#[actix_web::test]
async fn s5_without_a_ticket_id_the_customer_gets_their_waiting_ticket() {
    let db = TestDb::start().await;
    s5_ticket(
        &db,
        1,
        "WAITING",
        None,
        "(now() AT TIME ZONE 'Asia/Bangkok')::date",
    )
    .await;
    let alice = s5_ticket(
        &db,
        2,
        "WAITING",
        Some("customer-alice"),
        "(now() AT TIME ZONE 'Asia/Bangkok')::date",
    )
    .await;

    let ans = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &s5_uri(),
        None,
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "{}", ans.body);
    assert_eq!(
        ans.body["data"]["ticketId"],
        alice.to_string(),
        "{}",
        ans.body
    );
    assert_eq!(ans.body["data"]["displayNumber"], "W002", "{}", ans.body);
    assert_eq!(ans.body["data"]["ticketsAhead"], 1, "{}", ans.body);
    db.finish().await;
}

/// B2: a customer whose only ticket is COMPLETED has no active ticket.
#[actix_web::test]
async fn s5_without_a_ticket_id_a_completed_ticket_is_not_answered() {
    let db = TestDb::start().await;
    s5_ticket(
        &db,
        1,
        "COMPLETED",
        Some("customer-alice"),
        "(now() AT TIME ZONE 'Asia/Bangkok')::date",
    )
    .await;

    let ans = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &s5_uri(),
        None,
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::NOT_FOUND, "{}", ans.body);
    refused_with(&ans, "noTicket", "no active ticket");
    db.finish().await;
}

/// B3: one customer never gets another customer's ticket.
#[actix_web::test]
async fn s5_without_a_ticket_id_another_customers_ticket_is_never_answered() {
    let db = TestDb::start().await;
    s5_ticket(
        &db,
        1,
        "WAITING",
        Some("customer-bob"),
        "(now() AT TIME ZONE 'Asia/Bangkok')::date",
    )
    .await;

    let ans = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &s5_uri(),
        None,
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::NOT_FOUND, "{}", ans.body);
    refused_with(&ans, "noTicket", "bob's ticket is not alice's");
    db.finish().await;
}

/// B4: a ticket of an earlier service date is not today's ticket.
#[actix_web::test]
async fn s5_without_a_ticket_id_yesterdays_ticket_is_not_answered() {
    let db = TestDb::start().await;
    s5_ticket(
        &db,
        1,
        "WAITING",
        Some("customer-alice"),
        "(now() AT TIME ZONE 'Asia/Bangkok')::date - 1",
    )
    .await;

    let ans = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &s5_uri(),
        None,
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::NOT_FOUND, "{}", ans.body);
    db.finish().await;
}

/// Unchanged: no session is still 401, and a malformed ticketId is still 400.
#[actix_web::test]
async fn s5_no_session_and_a_malformed_id_answer_as_before() {
    let db = TestDb::start().await;
    let ans = call(&db, None, Method::GET, &s5_uri(), None, None).await;
    assert_eq!(ans.status, StatusCode::UNAUTHORIZED, "{}", ans.body);

    let bad = format!("{}&ticketId=not-a-uuid", s5_uri());
    let ans = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &bad,
        None,
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "{}", ans.body);
    db.finish().await;
}
