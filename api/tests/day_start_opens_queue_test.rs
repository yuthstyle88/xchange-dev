//! #5540 (design `docs/design/walkin-queue.md` §11, owner's decision 2026-10-29 "เปิดวัน = เปิดคิวด้วย"; queue-tickets.md
//! Q8): starting the branch day (`dayStartService`, READY → OPEN) also opens the branch queue, in the same
//! transaction, so the teller's walk-in `serve` is not refused `queueClosed`.
//!
//! Written before the implementation by the ticket author (claude/2ca5874b). Do not edit, delete or weaken; add tests
//! in a file of your own. The harness is `walkin_serve_command_test.rs`'s, copied so that file stays untouched, with a
//! READY day and a closed queue; the day-start fixtures are `v3_branch_day_commands_test.rs`'s. The commands read
//! `Utc::now()`, so the branch's opening hours are set around the real clock.
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
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
const MANAGER_ID: &str = "00000000-0000-0000-0000-000000000020";
const FINANCE_ID: &str = "00000000-0000-0000-0000-000000000030";
const CUSTOMER_ALICE_ID: &str = "00000000-0000-0000-0000-000000000040";
const CUSTOMER_BOB_ID: &str = "00000000-0000-0000-0000-000000000050";

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
        let schema = unique("daystartq");
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

        // The branch day is READY and its queue is closed, as on MAIN in desk check round 4.
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'READY')")
            .bind(B01)
            .execute(&pool)
            .await
            .expect("insert ops_state READY");
        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
             VALUES ($1, $2::uuid, false, 1, 1)",
        )
        .bind(B01)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert a closed branch_service_status");

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

// ------------------------------------------------------------ fixtures

const SERVE: &str = "/api/queue/tickets/serve";

/// Opening hours every day, in UTC, from `open_min` minutes before now to `close_min` minutes after.
async fn hours_around_now(db: &TestDb, open_min: i64, close_min: i64) {
    let now = Utc::now();
    let open = (now - chrono::Duration::minutes(open_min))
        .format("%H:%M")
        .to_string();
    let close = (now + chrono::Duration::minutes(close_min))
        .format("%H:%M")
        .to_string();
    let day = json!([{ "open": open, "close": close }]);
    let hours = json!({
        "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": day
    });
    sqlx::query("UPDATE branches SET timezone = 'UTC', opening_hours = $1 WHERE id = $2")
        .bind(hours)
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("set the opening hours");
}

async fn serve(db: &TestDb, who: &str, key: &str) -> Answer {
    call(
        db,
        Some(who),
        Method::POST,
        SERVE,
        Some(json!({ "branch": B01, "customerId": null })),
        Some(key),
    )
    .await
}

async fn tickets(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE branch_id = $1")
        .bind(B01)
        .fetch_one(&db.pool)
        .await
        .expect("count tickets")
}

// ------------------------------------------------------------ day start

const START: &str = "/api/branch-day/start";
const MANAGER: &str = "BRANCH_MANAGER@XC-B01";
const QUEUE_CLOSE: &str = "/api/queue/close";
const RATE_VERSION: &str = "2026-10-11T01:02:03.456Z";

/// The branch's current rate publication carries `RATE_VERSION` (`dayStartService` checks it).
async fn rates_published(db: &TestDb) {
    sqlx::query(
        "UPDATE rate_publications SET published_at = $1::timestamptz \
         WHERE id = (SELECT current_publication_id FROM branches WHERE id = $2)",
    )
    .bind(RATE_VERSION)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("stamp the rate publication");
}

async fn start_day(db: &TestDb, rates_verified: bool) -> Answer {
    call(
        db,
        Some(MANAGER),
        Method::POST,
        START,
        Some(json!({
            "branch": B01,
            "equipmentReady": true,
            "ratesVerified": rates_verified,
            "rateVersion": RATE_VERSION
        })),
        Some(&unique("start")),
    )
    .await
}

/// `(queue_open, queue_version)` of the branch, or `None` when it has no row.
async fn queue(db: &TestDb) -> Option<(bool, i64)> {
    sqlx::query_as(
        "SELECT queue_open, queue_version FROM branch_service_status WHERE branch_id = $1",
    )
    .bind(B01)
    .fetch_optional(&db.pool)
    .await
    .expect("read branch_service_status")
}

async fn phase(db: &TestDb) -> String {
    sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
        .bind(B01)
        .fetch_one(&db.pool)
        .await
        .expect("read ops_state")
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn starting_the_day_opens_a_closed_queue_and_the_walk_in_is_served() {
    let db = TestDb::start().await;
    hours_around_now(&db, 60, 120).await;
    rates_published(&db).await;
    let refused = serve(&db, "TELLER@XC-B01", &unique("sale")).await;
    refused_with(
        &refused,
        "queueClosed",
        "before the day starts the queue is closed",
    );

    let a = start_day(&db, true).await;

    assert!(a.status.is_success(), "{} {}", a.status, a.text);
    assert_eq!(phase(&db).await, "OPEN");
    let (open, version) = queue(&db).await.expect("a status row");
    assert!(open, "the day start opened the queue");
    assert!(version > 1, "the queue's version moved: {version}");
    let served = serve(&db, "TELLER@XC-B01", &unique("sale")).await;
    assert_eq!(served.status, StatusCode::OK, "{}", served.text);
    assert_eq!(served.body["status"], "SERVING", "{}", served.text);
    assert_eq!(tickets(&db).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn starting_the_day_on_an_already_open_queue_keeps_it_open_and_still_moves_its_version() {
    let db = TestDb::start().await;
    hours_around_now(&db, 60, 120).await;
    rates_published(&db).await;
    sqlx::query("UPDATE branch_service_status SET queue_open = true WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("open the queue");

    let a = start_day(&db, true).await;

    assert!(a.status.is_success(), "{} {}", a.status, a.text);
    let (open, version) = queue(&db).await.expect("a status row");
    assert!(open);
    assert!(version > 1, "the queue's version moved: {version}");
    db.finish().await;
}

#[actix_web::test]
async fn starting_the_day_of_a_branch_with_no_status_row_creates_it_open() {
    let db = TestDb::start().await;
    hours_around_now(&db, 60, 120).await;
    rates_published(&db).await;
    sqlx::query("DELETE FROM branch_service_status WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("remove the status row");

    let a = start_day(&db, true).await;

    assert!(a.status.is_success(), "{} {}", a.status, a.text);
    assert_eq!(queue(&db).await.map(|(open, _)| open), Some(true));
    db.finish().await;
}

#[actix_web::test]
async fn a_refused_day_start_leaves_the_day_ready_and_the_queue_closed() {
    let db = TestDb::start().await;
    hours_around_now(&db, 60, 120).await;
    rates_published(&db).await;

    let a = start_day(&db, false).await;

    assert!(a.status.is_client_error(), "{} {}", a.status, a.text);
    assert_eq!(phase(&db).await, "READY");
    assert_eq!(queue(&db).await, Some((false, 1)));
    db.finish().await;
}

#[actix_web::test]
async fn closing_the_queue_after_the_day_started_still_refuses_a_walk_in() {
    let db = TestDb::start().await;
    hours_around_now(&db, 60, 120).await;
    rates_published(&db).await;
    let a = start_day(&db, true).await;
    assert!(a.status.is_success(), "{} {}", a.status, a.text);

    let closed = call(
        &db,
        Some(MANAGER),
        Method::POST,
        QUEUE_CLOSE,
        Some(json!({ "branch": B01 })),
        Some(&unique("close")),
    )
    .await;
    assert!(
        closed.status.is_success(),
        "{} {}",
        closed.status,
        closed.text
    );

    let refused = serve(&db, "TELLER@XC-B01", &unique("sale")).await;
    refused_with(
        &refused,
        "queueClosed",
        "the manager closed the queue after the start",
    );
    db.finish().await;
}
