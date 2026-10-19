//! S7 (#3410, design `docs/design/blueprint/section-b-gaps.md` §S7, amends `queue-tickets.md` Q4) up-front tests:
//! C08 — a signed-in customer takes a walk-in queue number from the app, `queueTicketTake`
//! (`POST /api/app/queue/tickets` `{branch}`, `CustomerSelf`, `Idempotency-Key`), answering `MyQueueTicketData`.
//!
//! - B1: queue open, no ticket ⇒ one `WALK_IN` ticket with the customer's `customer_id`, the next sequence.
//! - B2: the same key again ⇒ the same ticket, no second row.
//! - B3: a new key while the first ticket is WAITING ⇒ 409 `alreadyQueued`, naming the first ticket's id.
//! - B4: two concurrent takes with different keys ⇒ exactly one ticket, the other 409 `alreadyQueued`; and the
//!   migration's partial unique index itself refuses a second active WALK_IN row for one customer, branch and day.
//! - B5: the queue is closed ⇒ 409 `queueClosed`, nothing written.
//! - B6: after the first ticket COMPLETES ⇒ a new take succeeds.
//! - §8: no session ⇒ 401; a staff session ⇒ 403.
//!
//! The owner-question in §S7 (a check-in on arrival, distance or rate limits) is answered by the merged design's
//! proposal: none in this ticket. So no test asks for one.
//! Every test fails on `main` by its result (the route answers 404, or the second index insert succeeds), not by a
//! compile error. The harness is `queue_commands_views_test.rs`'s, except that a customer's actor carries the seeded
//! branch's organisation (`ORG`), so no take can be refused for the fixture's sake. Needs `TEST_DATABASE_URL`.

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
        let schema = unique("s7take");
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
            ORG,
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

fn legacy_of(spec: &str) -> Sessions {
    match spec.split_once('@') {
        None => Sessions {
            owner: None,
            staff: None,
        },
        Some(("CUSTOMER", _)) => Sessions {
            owner: None,
            staff: None,
        },
        Some((_, "*")) => Sessions {
            owner: Some(Actor::owner("owner")),
            staff: None,
        },
        Some(("TELLER", branch)) => Sessions {
            owner: None,
            staff: Actor::staff(TELLER_ID.to_string(), OldRole::Front, BranchId::new(branch)),
        },
        Some(("BRANCH_MANAGER", branch)) => Sessions {
            owner: None,
            staff: Actor::staff(
                MANAGER_ID.to_string(),
                OldRole::Manager,
                BranchId::new(branch),
            ),
        },
        Some((_, branch)) => Sessions {
            owner: None,
            staff: Actor::staff(
                FINANCE_ID.to_string(),
                OldRole::Finance,
                BranchId::new(branch),
            ),
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

/// A domain refusal with `code` and a client-error status.
fn refused_with(a: &Answer, code: &str, what: &str) {
    assert!(
        a.status.is_client_error() && a.body["error"]["code"] == code,
        "{what}: expected a client-error {code} refusal; got {} {}",
        a.status,
        a.text
    );
}

const TAKE: &str = "/api/app/queue/tickets";
const ALICE: &str = "CUSTOMER@customer-alice";
const ALICE_CUSTOMER_ID: &str = "customer-alice";

fn branch_body() -> Value {
    json!({ "branch": B01 })
}

async fn take(db: &TestDb, who: &str, key: &str) -> Answer {
    call(
        db,
        Some(who),
        Method::POST,
        TAKE,
        Some(branch_body()),
        Some(key),
    )
    .await
}

/// The customer's tickets at B01 today: (id, service_type, status, sequence).
async fn alices_tickets(db: &TestDb) -> Vec<(Uuid, String, String, i32)> {
    sqlx::query_as(
        "SELECT id, service_type, status, sequence FROM queue_tickets \
         WHERE branch_id = $1 AND customer_id = $2 ORDER BY sequence",
    )
    .bind(B01)
    .bind(ALICE_CUSTOMER_ID)
    .fetch_all(&db.pool)
    .await
    .expect("read the customer's tickets")
}

/// A WALK_IN row written straight into `queue_tickets` (no issuer, no `queue_sequences`), for the index test only.
async fn seed_walk_in(db: &TestDb, sequence: i32, customer: Option<&str>, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
         display_number, status, customer_id, issued_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, (now() AT TIME ZONE 'Asia/Bangkok')::date, 'WALK_IN', $4, $5, $6, $7, now(), $8)",
    )
    .bind(id)
    .bind(ORG)
    .bind(B01)
    .bind(sequence)
    .bind(format!("W{sequence:03}"))
    .bind(status)
    .bind(customer)
    .bind(unique("seed"))
    .execute(&db.pool)
    .await
    .expect("seed a walk-in ticket");
    id
}

fn ticket_id(a: &Answer) -> String {
    a.body["ticketId"].as_str().unwrap_or_default().to_owned()
}

/// B1: one WALK_IN ticket for the customer, next in sequence after the desk's walk-in.
#[actix_web::test]
async fn s7_b1_a_customer_takes_one_walk_in_ticket_with_the_next_sequence() {
    let db = TestDb::start().await;
    let desk = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/queue/tickets",
        Some(branch_body()),
        Some(&unique("k")),
    )
    .await;
    assert_eq!(
        desk.status,
        StatusCode::OK,
        "B1 setup: the desk issues walk-in 1: {}",
        desk.text
    );
    let a = take(&db, ALICE, &unique("k")).await;
    assert_eq!(a.status, StatusCode::OK, "B1: {}", a.text);
    assert_eq!(a.body["branch"], json!(B01), "B1 branch: {}", a.text);
    assert_eq!(
        a.body["serviceType"],
        json!("WALK_IN"),
        "B1 serviceType: {}",
        a.text
    );
    assert_eq!(a.body["status"], json!("WAITING"), "B1 status: {}", a.text);
    assert_eq!(
        a.body["ticketsAhead"],
        json!(1),
        "B1 the desk's walk-in is ahead: {}",
        a.text
    );
    assert!(
        !a.body["displayNumber"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "B1 displayNumber: {}",
        a.text
    );
    let rows = alices_tickets(&db).await;
    assert_eq!(rows.len(), 1, "B1 one ticket for the customer: {rows:?}");
    let (id, service_type, status, sequence) = &rows[0];
    assert_eq!(
        id.to_string(),
        ticket_id(&a),
        "B1 the answer is the stored ticket"
    );
    assert_eq!(
        (service_type.as_str(), status.as_str(), *sequence),
        ("WALK_IN", "WAITING", 2),
        "B1 row: {rows:?}"
    );
    db.finish().await;
}

/// B2: a replay with the same key answers the same ticket and writes no second row.
#[actix_web::test]
async fn s7_b2_the_same_key_answers_the_same_ticket() {
    let db = TestDb::start().await;
    let key = unique("k");
    let first = take(&db, ALICE, &key).await;
    assert_eq!(first.status, StatusCode::OK, "B2 first: {}", first.text);
    let again = take(&db, ALICE, &key).await;
    assert_eq!(again.status, StatusCode::OK, "B2 replay: {}", again.text);
    assert_eq!(ticket_id(&again), ticket_id(&first), "B2 the same ticket");
    assert_eq!(alices_tickets(&db).await.len(), 1, "B2 no second row");
    db.finish().await;
}

/// B3: a new key while the first ticket is WAITING is refused 409 `alreadyQueued`, naming the first ticket.
#[actix_web::test]
async fn s7_b3_a_new_key_while_waiting_is_already_queued() {
    let db = TestDb::start().await;
    let first = take(&db, ALICE, &unique("k")).await;
    assert_eq!(first.status, StatusCode::OK, "B3 first: {}", first.text);
    let first_id = ticket_id(&first);
    let second = take(&db, ALICE, &unique("k")).await;
    assert_eq!(second.status, StatusCode::CONFLICT, "B3: {}", second.text);
    refused_with(&second, "alreadyQueued", "B3");
    assert!(
        !first_id.is_empty() && second.text.contains(&first_id),
        "B3 the refusal names the first ticket {first_id}: {}",
        second.text
    );
    assert_eq!(alices_tickets(&db).await.len(), 1, "B3 nothing written");
    db.finish().await;
}

/// B4: two concurrent takes with different keys make exactly one ticket.
#[actix_web::test]
async fn s7_b4_two_concurrent_takes_make_exactly_one_ticket() {
    let db = TestDb::start().await;
    let (k1, k2) = (unique("k"), unique("k"));
    let (a, b) = tokio::join!(take(&db, ALICE, &k1), take(&db, ALICE, &k2));
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "B4: {} | {}",
        a.text,
        b.text
    );
    let loser = if a.status == StatusCode::CONFLICT {
        &a
    } else {
        &b
    };
    refused_with(loser, "alreadyQueued", "B4 the loser");
    assert_eq!(alices_tickets(&db).await.len(), 1, "B4 exactly one ticket");
    db.finish().await;
}

/// B4 at the database: the migration's partial unique index refuses a second active WALK_IN row for one
/// customer, branch and day, and allows one once the first is no longer active.
#[actix_web::test]
async fn s7_b4_the_index_allows_one_active_walk_in_per_customer_branch_and_day() {
    let db = TestDb::start().await;
    let first = seed_walk_in(&db, 1, Some(ALICE_CUSTOMER_ID), "WAITING").await;
    let id = Uuid::new_v4();
    let second = sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
         display_number, status, customer_id, issued_at, idempotency_key) \
         SELECT $1, organization_id, branch_id, service_date, 'WALK_IN', 2, 'W002', 'CALLED', customer_id, now(), $2 \
         FROM queue_tickets WHERE id = $3",
    )
    .bind(id)
    .bind(unique("seed"))
    .bind(first)
    .execute(&db.pool)
    .await;
    let err = second
        .expect_err("a second active WALK_IN for the same customer, branch and day is refused");
    let code = err
        .as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned());
    assert_eq!(code.as_deref(), Some("23505"), "a unique violation: {err}");
    sqlx::query(
        "UPDATE queue_tickets SET status = 'COMPLETED', completed_at = now() WHERE id = $1",
    )
    .bind(first)
    .execute(&db.pool)
    .await
    .expect("complete the first");
    seed_walk_in(&db, 3, Some(ALICE_CUSTOMER_ID), "WAITING").await;
    seed_walk_in(&db, 4, None, "WAITING").await;
    seed_walk_in(&db, 5, None, "WAITING").await;
    db.finish().await;
}

/// B5: a closed queue refuses 409 `queueClosed` and writes nothing.
#[actix_web::test]
async fn s7_b5_a_closed_queue_refuses_and_writes_nothing() {
    let db = TestDb::start().await;
    sqlx::query("UPDATE branch_service_status SET queue_open = false WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("close the queue");
    let a = take(&db, ALICE, &unique("k")).await;
    assert_eq!(a.status, StatusCode::CONFLICT, "B5: {}", a.text);
    refused_with(&a, "queueClosed", "B5");
    assert!(alices_tickets(&db).await.is_empty(), "B5 nothing written");
    db.finish().await;
}

/// B6: once the first ticket COMPLETES, a new take succeeds with a new ticket.
#[actix_web::test]
async fn s7_b6_after_the_ticket_completes_a_new_take_succeeds() {
    let db = TestDb::start().await;
    let first = take(&db, ALICE, &unique("k")).await;
    assert_eq!(first.status, StatusCode::OK, "B6 first: {}", first.text);
    sqlx::query("UPDATE queue_tickets SET status = 'COMPLETED', served_at = now(), completed_at = now() WHERE customer_id = $1")
        .bind(ALICE_CUSTOMER_ID)
        .execute(&db.pool)
        .await
        .expect("complete the first ticket");
    let second = take(&db, ALICE, &unique("k")).await;
    assert_eq!(second.status, StatusCode::OK, "B6 second: {}", second.text);
    assert_ne!(ticket_id(&second), ticket_id(&first), "B6 a new ticket");
    assert_eq!(
        alices_tickets(&db).await.len(),
        2,
        "B6 two rows, one COMPLETED"
    );
    db.finish().await;
}

/// §8: no session ⇒ 401; a staff session ⇒ 403; neither writes a ticket.
#[actix_web::test]
async fn s7_only_a_signed_in_customer_may_take() {
    let db = TestDb::start().await;
    let nobody = call(
        &db,
        None,
        Method::POST,
        TAKE,
        Some(branch_body()),
        Some(&unique("k")),
    )
    .await;
    assert_eq!(
        nobody.status,
        StatusCode::UNAUTHORIZED,
        "no session: {}",
        nobody.text
    );
    let teller = take(&db, "TELLER@XC-B01", &unique("k")).await;
    assert_eq!(
        teller.status,
        StatusCode::FORBIDDEN,
        "staff: {}",
        teller.text
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE branch_id = $1")
        .bind(B01)
        .fetch_one(&db.pool)
        .await
        .expect("count tickets");
    assert_eq!(count, 0, "no ticket written");
    db.finish().await;
}
