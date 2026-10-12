//! Specification tests for QU-7 (docs/design/blueprint/queue-tickets.md §4.4, §4.5,
//! row QU-7 :301): queue commands, queue open/close, the views `queue_board`,
//! `branch_service_status`, `my_queue_ticket`, and the kiosk `confirm` issuing a ticket.
//!
//! Written before the implementation by the test-writer (gemini/studio-11, ordered by
//! claude/ad05bf8d, Rule A). Do not edit, delete or weaken.
//!
//! Rule A requirements:
//! 1. Every command's route answers, and refuses without its permission (QueueOperate;
//!    queueOpen/queueClose need BranchDayOperate) with the policy code main gives
//!    (roleNotAllowedHere), and signed-out gives signIn.
//! 2. queue_board: WAITING per type in call order, CALLED/SERVING with the teller,
//!    my counter state, and since -> {changed: false}.
//! 3. branch_service_status: public, with the Q14 fields, and carries no customer fields.
//! 4. my_queue_ticket: only the ticket's own customer.
//! 5. kiosk confirm issues exactly one walk-in ticket in the same transaction, and is
//!    refused queueClosed while the queue is closed.
//! 6. Idempotency-Key replay issues nothing new.
//!
//! Needs TEST_DATABASE_URL; nothing skips.
//!
//! Amended by the test-writer claude/5d45b017 for claude/dell-claude-3's review (#1266, 08:42Z), on
//! claude/ad05bf8d's order:
//! - gap 1: a policy refusal is pinned by its code (`roleNotAllowedHere`) and a client-error status,
//!   the status main's seam gives (`PolicyError::status`, 400), as `tx4_settings_routes_test.rs` does
//!   (lane controller's ruling on #672, issuecomment-6104304961);
//! - gap 2: `ticket_commands_change_the_stored_state` seeds tickets and calls each command on a real
//!   ticket, pinning QU-3's desk (`xc-queue/src/application/desk/queue_desk.rs`), `CounterService`
//!   and `StatusService`: what they store, plus `notFound`, `staleVersion` and `nothingWaiting`. The
//!   route loop no longer asks a made-up ticket id for success;
//! - gap 3: the replay test asserts the first call issued exactly one ticket and answered it;
//! - gap 4: the board seeds a SERVING ticket (another teller's) and pins it with its teller;
//! - fixture: a staff actor's organisation is the seeded branch's (`ORG`), not "XC". QU-3's desk
//!   authorises against the branch's organisation (`queue_desk.rs:186-191`), so an actor of another
//!   organisation is refused `outOfScope` there whatever the route does.

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
use xc_auth::domain::access::{Actor as User, Permission, Role, ScopedRole};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
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

#[actix_web::test]
async fn command_routes_answer_and_refuse_without_permission() {
    let db = TestDb::start().await;

    let queue_commands = [
        ("POST", "/api/queue/tickets"),
        ("POST", "/api/queue/call-next"),
        (
            "POST",
            "/api/queue/tickets/00000000-0000-0000-0000-000000000099/call",
        ),
        (
            "POST",
            "/api/queue/tickets/00000000-0000-0000-0000-000000000099/recall",
        ),
        (
            "POST",
            "/api/queue/tickets/00000000-0000-0000-0000-000000000099/skip",
        ),
        (
            "POST",
            "/api/queue/tickets/00000000-0000-0000-0000-000000000099/start",
        ),
        (
            "POST",
            "/api/queue/tickets/00000000-0000-0000-0000-000000000099/finish",
        ),
        (
            "POST",
            "/api/queue/tickets/00000000-0000-0000-0000-000000000099/cancel",
        ),
        ("POST", "/api/queue/counter/pause"),
        ("POST", "/api/queue/counter/resume"),
    ];

    let day_commands = [("POST", "/api/queue/open"), ("POST", "/api/queue/close")];

    let body = Some(json!({ "branch": B01 }));

    // 1. Queue commands (QueueOperate):
    for &(m, route) in &queue_commands {
        let method = Method::from_str(m).unwrap();

        // Signed out -> 401 signIn
        let ans = call(
            &db,
            None,
            method.clone(),
            route,
            body.clone(),
            Some("k-anon"),
        )
        .await;
        assert_eq!(
            ans.status,
            StatusCode::UNAUTHORIZED,
            "{route} signed-out status; got: {}",
            ans.text
        );
        assert_eq!(
            ans.body["error"]["code"], "signIn",
            "{route} signed-out code; got: {}",
            ans.text
        );

        // Finance (no QueueOperate) -> 403 roleNotAllowedHere
        let ans = call(
            &db,
            Some("FINANCE@XC-B01"),
            method.clone(),
            route,
            body.clone(),
            Some("k-finance"),
        )
        .await;
        policy_refused(&ans, &format!("{route} finance status"));

        // Teller (holds QueueOperate) -> the route exists and its permission gate lets the
        // teller through. What each command does is pinned on real tickets in
        // `ticket_commands_change_the_stored_state` (gap 2: no made-up ticket id must succeed).
        let ans = call(
            &db,
            Some("TELLER@XC-B01"),
            method.clone(),
            route,
            body.clone(),
            Some(&unique("k-teller")),
        )
        .await;
        assert!(
            ans.status != StatusCode::NOT_FOUND || ans.body["error"]["code"] == "notFound",
            "{route} teller: the route is served; got status {} and body: {}",
            ans.status,
            ans.text
        );
        assert!(
            ans.status != StatusCode::UNAUTHORIZED
                && !matches!(
                    ans.body["error"]["code"].as_str(),
                    Some("roleNotAllowed" | "roleNotAllowedHere" | "signIn")
                ),
            "{route} teller passes the permission gate; got status {} and body: {}",
            ans.status,
            ans.text
        );
    }

    // 2. Day commands (BranchDayOperate):
    for &(m, route) in &day_commands {
        let method = Method::from_str(m).unwrap();

        // Signed out -> 401 signIn
        let ans = call(
            &db,
            None,
            method.clone(),
            route,
            body.clone(),
            Some("k-anon"),
        )
        .await;
        assert_eq!(
            ans.status,
            StatusCode::UNAUTHORIZED,
            "{route} signed-out status; got: {}",
            ans.text
        );
        assert_eq!(
            ans.body["error"]["code"], "signIn",
            "{route} signed-out code; got: {}",
            ans.text
        );

        // Finance -> 403 roleNotAllowedHere
        let ans = call(
            &db,
            Some("FINANCE@XC-B01"),
            method.clone(),
            route,
            body.clone(),
            Some("k-finance"),
        )
        .await;
        policy_refused(&ans, &format!("{route} finance status"));

        // Teller (holds QueueOperate but NOT BranchDayOperate) -> 403 roleNotAllowedHere
        let ans = call(
            &db,
            Some("TELLER@XC-B01"),
            method.clone(),
            route,
            body.clone(),
            Some("k-teller-day"),
        )
        .await;
        policy_refused(&ans, &format!("{route} teller without BranchDayOperate"));

        // Branch Manager (holds BranchDayOperate) -> answers
        let ans = call(
            &db,
            Some("BRANCH_MANAGER@XC-B01"),
            method.clone(),
            route,
            body.clone(),
            Some(&unique("k-bm")),
        )
        .await;
        assert!(
            ans.status.is_success(),
            "{route} branch manager answer; got status {} and body: {}",
            ans.status,
            ans.text
        );
    }

    db.finish().await;
}

/// Gap 2 of claude/dell-claude-3's review: each command on a real ticket, pinned to what QU-3's
/// `QueueDesk` (`call_next` :200, `call` :262, `recall` :325, `skip` :366, `start` :423, `finish` :482,
/// `cancel` :541), `CounterService::pause`/`resume` and `StatusService::open`/`close` store.
#[actix_web::test]
async fn ticket_commands_change_the_stored_state() {
    let db = TestDb::start().await;
    let teller = Some("TELLER@XC-B01");
    let manager = Some("BRANCH_MANAGER@XC-B01");
    let branch_only = || Some(json!({ "branch": B01 }));
    let post = |who: Option<&'static str>, route: String, body: Option<Value>| {
        let db = &db;
        async move {
            call(
                db,
                who,
                Method::POST,
                &route,
                body,
                Some(&unique("k-effect")),
            )
            .await
        }
    };

    // A BOOKING ticket with a later sequence than two WALK_INs: call order takes it first (QU-3).
    blueprint_booking(&db.pool, B01, "BK-E1").await;
    let (b7, w1, w2) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key, booking_id) \
         VALUES \
           ($1, $4::uuid, $5, current_date, 'BOOKING', 7, 'B007', 'WAITING', now() - interval '1m', 'key-e-b7', 'BK-E1'), \
           ($2, $4::uuid, $5, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now() - interval '9m', 'key-e-w1', NULL), \
           ($3, $4::uuid, $5, current_date, 'WALK_IN', 2, 'W002', 'WAITING', now() - interval '8m', 'key-e-w2', NULL)",
    )
    .bind(b7)
    .bind(w1)
    .bind(w2)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert waiting tickets");
    let events = |id: Uuid, kind: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM queue_events WHERE ticket_id = $1 AND kind = $2",
            )
            .bind(id)
            .bind(kind)
            .fetch_one(&pool)
            .await
            .expect("count events")
        }
    };

    // call-next: the first in call order (BOOKING before WALK_IN), CALLED by the caller.
    let ans = post(teller, "/api/queue/call-next".into(), branch_only()).await;
    assert!(
        ans.status.is_success(),
        "call-next: {} {}",
        ans.status,
        ans.text
    );
    assert!(
        ans.text.contains("B007"),
        "call-next answers the ticket it called: {}",
        ans.text
    );
    assert_eq!(
        ticket_row(&db, b7).await,
        ("CALLED".into(), Some(TELLER_ID.into()), None)
    );
    assert_eq!(
        ticket_row(&db, w1).await.0,
        "WAITING",
        "the walk-ins still wait"
    );
    assert_eq!(events(b7, "CALL").await, 1);

    // recall: still CALLED, one RECALL event.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{b7}/recall"),
        branch_only(),
    )
    .await;
    assert!(
        ans.status.is_success(),
        "recall: {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ticket_row(&db, b7).await.0, "CALLED");
    assert_eq!(events(b7, "RECALL").await, 1, "recall writes one event");

    // skip: NO_SHOW, final.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{b7}/skip"),
        branch_only(),
    )
    .await;
    assert!(ans.status.is_success(), "skip: {} {}", ans.status, ans.text);
    assert_eq!(ticket_row(&db, b7).await.0, "NO_SHOW");

    // call: the chosen ticket (W002, not the first in call order), CALLED by the caller.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{w2}/call"),
        branch_only(),
    )
    .await;
    assert!(ans.status.is_success(), "call: {} {}", ans.status, ans.text);
    assert_eq!(
        ticket_row(&db, w2).await,
        ("CALLED".into(), Some(TELLER_ID.into()), None)
    );
    assert_eq!(
        ticket_row(&db, w1).await.0,
        "WAITING",
        "call takes the chosen ticket only"
    );

    // start: SERVING; a second start is staleVersion and changes nothing.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{w2}/start"),
        branch_only(),
    )
    .await;
    assert!(
        ans.status.is_success(),
        "start: {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ticket_row(&db, w2).await.0, "SERVING");
    let ans = post(
        teller,
        format!("/api/queue/tickets/{w2}/start"),
        branch_only(),
    )
    .await;
    refused_with(&ans, "staleVersion", "start on a SERVING ticket");
    assert_eq!(ticket_row(&db, w2).await.0, "SERVING");

    // finish: COMPLETED.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{w2}/finish"),
        branch_only(),
    )
    .await;
    assert!(
        ans.status.is_success(),
        "finish: {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ticket_row(&db, w2).await.0, "COMPLETED");

    // cancel: a WAITING ticket becomes CANCELLED with the reason given.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{w1}/cancel"),
        Some(json!({ "branch": B01, "reason": "customerLeft" })),
    )
    .await;
    assert!(
        ans.status.is_success(),
        "cancel: {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(
        ticket_row(&db, w1).await,
        ("CANCELLED".into(), None, Some("customerLeft".into()))
    );

    // call-next with nothing waiting: nothingWaiting.
    let ans = post(teller, "/api/queue/call-next".into(), branch_only()).await;
    refused_with(&ans, "nothingWaiting", "call-next with nothing waiting");

    // a ticket that does not exist: notFound.
    let ans = post(
        teller,
        format!("/api/queue/tickets/{}/call", Uuid::new_v4()),
        branch_only(),
    )
    .await;
    refused_with(&ans, "notFound", "call on an unknown ticket");

    // pause / resume: the caller's counter.
    let counter = || {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT status FROM queue_counters WHERE branch_id = $1 AND teller_id = $2::uuid",
            )
            .bind(B01)
            .bind(TELLER_ID)
            .fetch_one(&pool)
            .await
            .expect("read the counter")
        }
    };
    let ans = post(teller, "/api/queue/counter/pause".into(), branch_only()).await;
    assert!(
        ans.status.is_success(),
        "pause: {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(counter().await, "PAUSED");
    let ans = post(teller, "/api/queue/counter/resume".into(), branch_only()).await;
    assert!(
        ans.status.is_success(),
        "resume: {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(counter().await, "ACTIVE");

    // close / open: branch_service_status.queue_open.
    let queue_open = || {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT queue_open FROM branch_service_status WHERE branch_id = $1",
            )
            .bind(B01)
            .fetch_one(&pool)
            .await
            .expect("read the status row")
        }
    };
    let ans = post(manager, "/api/queue/close".into(), branch_only()).await;
    assert!(
        ans.status.is_success(),
        "close: {} {}",
        ans.status,
        ans.text
    );
    assert!(!queue_open().await, "close closes the queue");
    let ans = post(manager, "/api/queue/open".into(), branch_only()).await;
    assert!(ans.status.is_success(), "open: {} {}", ans.status, ans.text);
    assert!(queue_open().await, "open opens it again");

    db.finish().await;
}

#[actix_web::test]
async fn queue_board_view_returns_waiting_in_call_order_called_and_my_counter() {
    let db = TestDb::start().await;

    // Seed 4 waiting tickets:
    // W001 sequence 1, B002 sequence 2, W003 sequence 3, B001 sequence 1
    // Call order rule: BOOKING first (sorted by sequence asc), then WALK_IN (sorted by sequence asc).
    // Expected order: B001, B002, W001, W003.
    let t_w1 = Uuid::new_v4();
    let t_b2 = Uuid::new_v4();
    let t_w3 = Uuid::new_v4();
    let t_b1 = Uuid::new_v4();
    let t_called = Uuid::new_v4();

    // Insert dummy booking rows for the booking tickets
    blueprint_booking(&db.pool, B01, "BK-1").await;
    blueprint_booking(&db.pool, B01, "BK-2").await;

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key, booking_id) \
         VALUES \
           ($1, $5::uuid, $6, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now() - interval '10m', 'key-w1', NULL), \
           ($2, $5::uuid, $6, current_date, 'BOOKING', 2, 'B002', 'WAITING', now() - interval '8m', 'key-b2', 'BK-2'), \
           ($3, $5::uuid, $6, current_date, 'WALK_IN', 3, 'W003', 'WAITING', now() - interval '5m', 'key-w3', NULL), \
           ($4, $5::uuid, $6, current_date, 'BOOKING', 1, 'B001', 'WAITING', now() - interval '2m', 'key-b1', 'BK-1')",
    )
    .bind(t_w1)
    .bind(t_b2)
    .bind(t_w3)
    .bind(t_b1)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert waiting tickets");

    // Insert 1 CALLED ticket assigned to teller
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 99, 'W099', 'CALLED', $4::uuid, now() - interval '1m', now(), 'key-called')",
    )
    .bind(t_called)
    .bind(ORG)
    .bind(B01)
    .bind(TELLER_ID)
    .execute(&db.pool)
    .await
    .expect("insert called ticket");

    // Gap 4: one SERVING ticket, another teller's (the manager's).
    let t_serving = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, served_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 98, 'W098', 'SERVING', $4::uuid, now() - interval '3m', now() - interval '2m', now() - interval '1m', 'key-serving')",
    )
    .bind(t_serving)
    .bind(ORG)
    .bind(B01)
    .bind(MANAGER_ID)
    .execute(&db.pool)
    .await
    .expect("insert serving ticket");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");

    // Refuses signed-out
    let ans = call(&db, None, Method::GET, &uri, None, None).await;
    assert_eq!(
        ans.status,
        StatusCode::UNAUTHORIZED,
        "queue_board signed out: {}",
        ans.text
    );
    assert_eq!(ans.body["error"]["code"], "signIn");

    // Refuses unprivileged (Finance)
    let ans = call(&db, Some("FINANCE@XC-B01"), Method::GET, &uri, None, None).await;
    policy_refused(&ans, "queue_board finance");

    // Teller reads queue board
    let ans = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "queue_board teller: {}",
        ans.text
    );

    let data = &ans.body["data"];
    assert!(data.is_object(), "data is object: {}", ans.text);

    // Verify waiting array in call order: B001, B002, W001, W003
    let waiting = data["waiting"].as_array().expect("waiting is array");
    assert_eq!(waiting.len(), 4, "4 waiting tickets: {:?}", waiting);
    let order: Vec<&str> = waiting
        .iter()
        .map(|t| t["displayNumber"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        order,
        vec!["B001", "B002", "W001", "W003"],
        "call order: BOOKING asc, then WALK_IN asc"
    );

    // Verify called / serving tickets with teller
    let called = data["called"].as_array().expect("called is array");
    assert!(!called.is_empty(), "called tickets list not empty");
    let by_number = |n: &str| {
        called
            .iter()
            .find(|t| t["displayNumber"] == n)
            .unwrap_or_else(|| panic!("{n} is on the board's CALLED/SERVING list: {called:?}"))
    };
    let called_t = by_number("W099");
    assert_eq!(called_t["tellerId"], TELLER_ID);
    assert_eq!(called_t["status"], "CALLED");
    let serving_t = by_number("W098");
    assert_eq!(
        serving_t["tellerId"], MANAGER_ID,
        "the SERVING ticket with its teller"
    );
    assert_eq!(serving_t["status"], "SERVING");
    assert_eq!(
        called.len(),
        2,
        "only CALLED and SERVING tickets: {called:?}"
    );

    // Verify myCounter state
    assert_eq!(data["myCounter"]["status"], "ACTIVE");

    // since={version} -> { changed: false }
    let version = ans.body["version"].as_i64().expect("version integer");
    let uri_since = format!("/api/v3/views/queue_board?branch={B01}&since={version}");
    let ans_since = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::GET,
        &uri_since,
        None,
        None,
    )
    .await;
    assert_eq!(ans_since.status, StatusCode::OK);
    assert_eq!(
        ans_since.body["changed"], false,
        "since current version returns changed: false"
    );

    db.finish().await;
}

#[actix_web::test]
async fn branch_service_status_view_is_public_and_returns_q14_fields_without_customer_data() {
    let db = TestDb::start().await;

    // Seed branch_service_status row with Q14 fields
    sqlx::query(
        "UPDATE branch_service_status \
         SET queue_open = true, active_counters = 2, queue_version = 10, \
             estimate_seconds = 300, estimate_low_seconds = 210, estimate_high_seconds = 390, \
             estimate_as_of = '2026-11-02T10:00:00Z' \
         WHERE branch_id = $1",
    )
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("update branch_service_status");

    let uri = format!("/api/v3/views/branch_service_status?branch={B01}");

    // MUST be accessible without any session / header (public)
    let ans = call(&db, None, Method::GET, &uri, None, None).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "public access to branch_service_status: {}",
        ans.text
    );

    let data = &ans.body["data"];
    assert!(data.is_object(), "data is object: {}", ans.text);

    // Q14 fields
    assert_eq!(data["queueOpen"], true);
    assert_eq!(data["activeCounters"], 2);
    assert_eq!(data["estimateSeconds"], 300);
    assert_eq!(data["estimateLowSeconds"], 210);
    assert_eq!(data["estimateHighSeconds"], 390);
    assert!(data["estimateAsOf"].is_string());
    assert!(data.get("waiting").is_some(), "waiting counts present");

    // Privacy guard: ensure NO customer fields exist in data
    fn check_no_customer_keys(val: &Value) {
        match val {
            Value::Object(map) => {
                for (k, v) in map {
                    let lower = k.to_lowercase();
                    assert!(
                        !lower.contains("customer"),
                        "data contains customer key: {k}"
                    );
                    assert!(!lower.contains("passport"), "data contains passport: {k}");
                    assert!(!lower.contains("phone"), "data contains phone: {k}");
                    assert!(!lower.contains("name"), "data contains name: {k}");
                    check_no_customer_keys(v);
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    check_no_customer_keys(item);
                }
            }
            _ => {}
        }
    }
    check_no_customer_keys(data);

    // since={version} -> { changed: false }
    let version = ans.body["version"].as_i64().expect("version");
    let uri_since = format!("/api/v3/views/branch_service_status?branch={B01}&since={version}");
    let ans_since = call(&db, None, Method::GET, &uri_since, None, None).await;
    assert_eq!(ans_since.status, StatusCode::OK);
    assert_eq!(
        ans_since.body["changed"], false,
        "since current version returns changed: false"
    );

    db.finish().await;
}

#[actix_web::test]
async fn my_queue_ticket_view_accessible_only_by_ticket_customer() {
    let db = TestDb::start().await;

    let ticket_alice = Uuid::new_v4();
    let ticket_other_1 = Uuid::new_v4();
    let ticket_other_2 = Uuid::new_v4();

    // 2 earlier waiting tickets
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key) \
         VALUES \
           ($1, $4::uuid, $5, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now() - interval '5m', 'key-oth-1'), \
           ($2, $4::uuid, $5, current_date, 'WALK_IN', 2, 'W002', 'WAITING', now() - interval '4m', 'key-oth-2'), \
           ($3, $4::uuid, $5, current_date, 'WALK_IN', 3, 'W003', 'WAITING', now() - interval '3m', 'key-alice')",
    )
    .bind(ticket_other_1)
    .bind(ticket_other_2)
    .bind(ticket_alice)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert queue tickets");

    // Assign customer_id = 'customer-alice' to ticket_alice
    sqlx::query("UPDATE queue_tickets SET customer_id = 'customer-alice' WHERE id = $1")
        .bind(ticket_alice)
        .execute(&db.pool)
        .await
        .expect("set customer_id");

    let uri = format!("/api/v3/views/my_queue_ticket?branch={B01}&ticketId={ticket_alice}");

    // Signed out -> refused (401)
    let ans = call(&db, None, Method::GET, &uri, None, None).await;
    assert!(
        ans.status == StatusCode::UNAUTHORIZED || ans.status == StatusCode::FORBIDDEN,
        "signed-out caller refused: status {}",
        ans.status
    );

    // Another customer (customer-bob) -> refused (not their ticket)
    let ans = call(
        &db,
        Some("CUSTOMER@customer-bob"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    assert!(
        ans.status == StatusCode::FORBIDDEN
            || ans.status == StatusCode::NOT_FOUND
            || ans.status == StatusCode::BAD_REQUEST,
        "customer-bob refused access to alice's ticket: status {}",
        ans.status
    );

    // Ticket's own customer (customer-alice) -> 200 OK
    let ans = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "customer-alice receives ticket: {}",
        ans.text
    );

    let data = &ans.body["data"];
    assert_eq!(data["displayNumber"], "W003");
    assert_eq!(data["status"], "WAITING");
    assert_eq!(data["ticketsAhead"], 2, "2 tickets ahead of W003");
    assert!(data.get("estimateSeconds").is_some());
    assert!(data.get("estimateLowSeconds").is_some());
    assert!(data.get("estimateHighSeconds").is_some());
    assert!(
        data.get("estimateAsOf").is_some() || data.get("asOf").is_some(),
        "estimate timestamp present"
    );

    // since={version} -> { changed: false }
    let version = ans.body["version"].as_i64().expect("version");
    let uri_since = format!("{uri}&since={version}");
    let ans_since = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri_since,
        None,
        None,
    )
    .await;
    assert_eq!(ans_since.status, StatusCode::OK);
    assert_eq!(
        ans_since.body["changed"], false,
        "since current version returns changed: false"
    );

    db.finish().await;
}

async fn create_quote(db: &TestDb) -> String {
    let ans = call(
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
    assert_eq!(ans.status, StatusCode::OK, "create quote: {}", ans.text);
    let id = ans.body["id"].as_str().expect("quote id").to_string();
    sqlx::query("UPDATE kiosk_requests SET created_at = now() - interval '10s' WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .expect("update created_at");
    id
}

#[actix_web::test]
async fn kiosk_confirm_issues_exactly_one_walk_in_ticket_in_same_transaction_and_refuses_when_closed()
 {
    let db = TestDb::start().await;

    // Create quote while queue is still open (counter is open)
    let closed_req_id = create_quote(&db).await;

    // Part 1: When queue is CLOSED, kiosk confirm refuses with queueClosed and issues NO ticket
    sqlx::query("UPDATE branch_service_status SET queue_open = false WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("close queue");

    let ans = call(
        &db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "confirm",
            "payload": {
                "id": closed_req_id,
                "branchId": B01,
            },
        })),
        None,
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "kiosk confirm while queue closed: got {}",
        ans.text
    );
    assert!(
        ans.text.contains("queueClosed"),
        "error must mention queueClosed; got: {}",
        ans.text
    );

    // Verify 0 tickets issued
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE kiosk_request_id = $1")
            .bind(&closed_req_id)
            .fetch_one(&db.pool)
            .await
            .expect("count tickets");
    assert_eq!(count, 0, "no ticket issued when queue is closed");

    // Verify kiosk request state rolled back / remained QUOTED
    let req_status: String = sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE id = $1")
        .bind(&closed_req_id)
        .fetch_one(&db.pool)
        .await
        .expect("fetch request status");
    assert_eq!(
        req_status, "QUOTED",
        "kiosk request remains QUOTED after refusal"
    );

    // Part 2: When queue is OPEN, kiosk confirm succeeds and issues exactly ONE ticket in same tx
    sqlx::query("UPDATE branch_service_status SET queue_open = true WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("open queue");

    let open_req_id = create_quote(&db).await;

    let ans = call(
        &db,
        None,
        Method::POST,
        "/api/kiosk",
        Some(json!({
            "action": "confirm",
            "payload": {
                "id": open_req_id,
                "branchId": B01,
            },
        })),
        None,
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "kiosk confirm when queue open: got {}",
        ans.text
    );

    // Verify exactly 1 ticket issued for req-open-1
    let (t_count, s_type, s_status): (i64, String, String) = sqlx::query_as(
        "SELECT count(*), coalesce(max(service_type), ''), coalesce(max(status), '') \
         FROM queue_tickets WHERE kiosk_request_id = $1",
    )
    .bind(&open_req_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch ticket info");

    assert_eq!(t_count, 1, "exactly one ticket issued for kiosk request");
    assert_eq!(s_type, "WALK_IN", "ticket is WALK_IN");
    assert_eq!(s_status, "WAITING", "ticket status is WAITING");

    db.finish().await;
}

#[actix_web::test]
async fn idempotency_key_replay_issues_nothing_new() {
    let db = TestDb::start().await;

    let key = unique("idem-test-key");
    let body = Some(json!({ "branch": B01 }));

    // Initial issue
    let ans1 = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/queue/tickets",
        body.clone(),
        Some(&key),
    )
    .await;

    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "first call succeeds: {}",
        ans1.text
    );

    let count1: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE branch_id = $1")
        .bind(B01)
        .fetch_one(&db.pool)
        .await
        .expect("count tickets");
    // Gap 3: the first call issued exactly one WAITING walk-in, and its answer names it.
    assert_eq!(count1, 1, "the first call issues one ticket");
    let (issued_number, issued_type, issued_status): (String, String, String) = sqlx::query_as(
        "SELECT display_number, service_type, status FROM queue_tickets WHERE branch_id = $1",
    )
    .bind(B01)
    .fetch_one(&db.pool)
    .await
    .expect("the issued ticket");
    assert_eq!(
        (issued_type.as_str(), issued_status.as_str()),
        ("WALK_IN", "WAITING")
    );
    assert!(
        ans1.text.contains(&issued_number),
        "the answer names the issued ticket {issued_number}: {}",
        ans1.text
    );

    // Replay with identical key and body
    let ans2 = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/queue/tickets",
        body,
        Some(&key),
    )
    .await;

    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "replay call succeeds: {}",
        ans2.text
    );
    assert_eq!(
        ans1.body, ans2.body,
        "replay returns identical response body"
    );

    let count2: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE branch_id = $1")
        .bind(B01)
        .fetch_one(&db.pool)
        .await
        .expect("count tickets after replay");

    assert_eq!(
        count1, count2,
        "idempotent replay issues no new ticket in database"
    );

    db.finish().await;
}

/// N1 (claude/14cc24bd's re-review, accepted by the lane controller): the policy rows are named as
/// the design's §4.4 table names them, `[required]` by the lane controller's ruling
/// issuecomment-6107392168, all on `EntryPath::Rpc`. Each command's and the board's `INHERITS`
/// names its row, so a row under another name is refused or never asked; this pins the names.
#[test]
fn policy_rows_are_named_as_the_ruling_requires() {
    let queue_operate = [
        "ticketIssueWalkIn",
        "ticketCallNext",
        "ticketCall",
        "ticketRecall",
        "ticketSkip",
        "ticketStart",
        "ticketFinish",
        "ticketCancel",
        "counterPause",
        "counterResume",
        "queueBoard",
    ];
    for name in queue_operate {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(name)),
            Some(Need::Permission(Permission::QueueOperate)),
            "Rpc {name} needs QueueOperate"
        );
    }
    for name in ["queueOpen", "queueClose"] {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(name)),
            Some(Need::Permission(Permission::BranchDayOperate)),
            "Rpc {name} needs BranchDayOperate"
        );
    }
}

/// A CONFIRMED Blueprint booking `id` of `branch`, with its own customer and OPEN slot: `queue_tickets.booking_id`
/// names the new `bookings` (BK-1's `*_booking_slots.sql`).
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
