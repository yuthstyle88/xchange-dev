//! Specification tests for #3635 (§8 Behaviour table):
//! Verifies that queue calls write the "your number is called" notice
//! (`queue_called` template) to `notifications` through the HTTP routes.

mod common;

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
        let schema = unique("q_notif_test");
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
             VALUES ($1::uuid, $2::uuid, 'Teller 1', 'ACTIVE', 'test')",
        )
        .bind(TELLER_ID)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert teller user");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid)",
        )
        .bind(B01)
        .bind(TELLER_ID)
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
        .bind(TELLER_ID)
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
    Some(User::new(
        TELLER_ID,
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
        Some(("TELLER", branch)) => Sessions {
            owner: None,
            staff: Actor::staff(TELLER_ID.to_string(), OldRole::Front, BranchId::new(branch)),
        },
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
}

async fn call_route(
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

    let gen_key;
    let actual_key = match key {
        Some(k) => k,
        None => {
            gen_key = unique("k-req");
            &gen_key
        }
    };
    req = req.insert_header(("idempotency-key", actual_key));

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();

    Answer { status, text }
}

// ------------------------------------------------------------ seeds & queries

async fn seed_customer(db: &TestDb, user_id: Option<Uuid>) -> (String, Option<Uuid>) {
    let customer_id = unique("CUST");
    if let Some(uid) = user_id {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'Customer User', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(uid)
        .bind(ORG)
        .execute(&db.pool)
        .await
        .expect("insert customer user");

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, user_id, created_at) \
             VALUES ($1, $2::uuid, $3, 'Customer Name', 'LIVE', $4, now())",
        )
        .bind(&customer_id)
        .bind(ORG)
        .bind(&customer_id)
        .bind(uid)
        .execute(&db.pool)
        .await
        .expect("insert customer with user");
    } else {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, user_id, created_at) \
             VALUES ($1, $2::uuid, $3, 'Customer Name', 'LIVE', NULL, now())",
        )
        .bind(&customer_id)
        .bind(ORG)
        .bind(&customer_id)
        .execute(&db.pool)
        .await
        .expect("insert customer without user");
    }
    (customer_id, user_id)
}

async fn seed_waiting_ticket(
    db: &TestDb,
    customer_id: Option<&str>,
    sequence: i32,
    display_number: &str,
) -> Uuid {
    let ticket_id = Uuid::new_v4();
    let key = unique("k-ticket");
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key, customer_id) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', $4, $5, 'WAITING', now(), $6, $7)",
    )
    .bind(ticket_id)
    .bind(ORG)
    .bind(B01)
    .bind(sequence)
    .bind(display_number)
    .bind(key)
    .bind(customer_id)
    .execute(&db.pool)
    .await
    .expect("insert waiting ticket");
    ticket_id
}

#[derive(sqlx::FromRow, Debug)]
struct NotificationRow {
    event_id: String,
    template_key: String,
    recipient_user_id: Uuid,
}

async fn fetch_notifications(db: &TestDb) -> Vec<NotificationRow> {
    sqlx::query_as::<_, NotificationRow>(
        "SELECT event_id, template_key, recipient_user_id FROM notifications ORDER BY seq",
    )
    .fetch_all(&db.pool)
    .await
    .expect("fetch notifications")
}

// ------------------------------------------------------------ behaviour table (§8)

/// Row 1: POST /api/queue/call-next -> 200; exactly one notifications row:
/// template_key = 'queue_called', recipient_user_id = the customer's user, event_id = 'queue.called:<ticket>:1'
#[actix_web::test]
async fn row_1_call_next_writes_notification() {
    let db = TestDb::start().await;
    let cust_user = Uuid::new_v4();
    let (cid, _) = seed_customer(&db, Some(cust_user)).await;
    let tid = seed_waiting_ticket(&db, Some(&cid), 1, "W001").await;

    let ans = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/queue/call-next",
        Some(json!({ "branch": B01 })),
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200; got {}", ans.text);

    let rows = fetch_notifications(&db).await;
    assert_eq!(
        rows.len(),
        1,
        "row 1 requires exactly one notifications row; found {}",
        rows.len()
    );
    assert_eq!(rows[0].template_key, "queue_called");
    assert_eq!(rows[0].recipient_user_id, cust_user);
    assert_eq!(rows[0].event_id, format!("queue.called:{tid}:1"));

    db.finish().await;
}

/// Row 2: then POST /api/queue/tickets/{id}/recall -> 200; a second row, event_id ending :2
#[actix_web::test]
async fn row_2_recall_writes_second_notification() {
    let db = TestDb::start().await;
    let cust_user = Uuid::new_v4();
    let (cid, _) = seed_customer(&db, Some(cust_user)).await;
    let tid = seed_waiting_ticket(&db, Some(&cid), 1, "W001").await;

    let ans1 = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/queue/call-next",
        Some(json!({ "branch": B01 })),
        None,
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "expected 200; got {}",
        ans1.text
    );

    let ans2 = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/queue/tickets/{tid}/recall"),
        Some(json!({ "branch": B01 })),
        None,
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "expected 200; got {}",
        ans2.text
    );

    let rows = fetch_notifications(&db).await;
    assert_eq!(
        rows.len(),
        2,
        "row 2 requires a second notifications row; found {}",
        rows.len()
    );
    assert_eq!(rows[1].template_key, "queue_called");
    assert_eq!(rows[1].recipient_user_id, cust_user);
    assert!(
        rows[1].event_id.ends_with(":2"),
        "row 2 event_id must end in :2; got {}",
        rows[1].event_id
    );
    assert_eq!(rows[1].event_id, format!("queue.called:{tid}:2"));

    db.finish().await;
}

/// Row 3: POST /api/queue/tickets/{id}/call on a chosen WAITING ticket -> 200; one row for that ticket
#[actix_web::test]
async fn row_3_call_writes_notification() {
    let db = TestDb::start().await;
    let cust_user = Uuid::new_v4();
    let (cid, _) = seed_customer(&db, Some(cust_user)).await;
    let tid = seed_waiting_ticket(&db, Some(&cid), 1, "W001").await;

    let ans = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/queue/tickets/{tid}/call"),
        Some(json!({ "branch": B01 })),
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200; got {}", ans.text);

    let rows = fetch_notifications(&db).await;
    assert_eq!(
        rows.len(),
        1,
        "row 3 requires one notifications row for that ticket; found {}",
        rows.len()
    );
    assert_eq!(rows[0].template_key, "queue_called");
    assert_eq!(rows[0].recipient_user_id, cust_user);
    assert_eq!(rows[0].event_id, format!("queue.called:{tid}:1"));

    db.finish().await;
}

/// Row 4: a ticket with no customer_id (walk-in), called -> 200; no notifications row (unchanged)
#[actix_web::test]
async fn row_4_walk_in_without_customer_id_writes_no_notification() {
    let db = TestDb::start().await;
    let tid = seed_waiting_ticket(&db, None, 1, "W001").await;

    let ans = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/queue/tickets/{tid}/call"),
        Some(json!({ "branch": B01 })),
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200; got {}", ans.text);

    let rows = fetch_notifications(&db).await;
    assert_eq!(
        rows.len(),
        0,
        "row 4 requires no notifications row; found {}",
        rows.len()
    );

    db.finish().await;
}

/// Row 5: a customer with no user_id, called -> 200; no row (unchanged)
#[actix_web::test]
async fn row_5_customer_without_user_id_writes_no_notification() {
    let db = TestDb::start().await;
    let (cid, _) = seed_customer(&db, None).await;
    let tid = seed_waiting_ticket(&db, Some(&cid), 1, "W001").await;

    let ans = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/queue/tickets/{tid}/call"),
        Some(json!({ "branch": B01 })),
        None,
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200; got {}", ans.text);

    let rows = fetch_notifications(&db).await;
    assert_eq!(
        rows.len(),
        0,
        "row 5 requires no notifications row; found {}",
        rows.len()
    );

    db.finish().await;
}

/// Row 6: the same call sent twice with one Idempotency-Key -> one row; the replay writes nothing
#[actix_web::test]
async fn row_6_idempotent_replay_writes_nothing() {
    let db = TestDb::start().await;
    let cust_user = Uuid::new_v4();
    let (cid, _) = seed_customer(&db, Some(cust_user)).await;
    let tid = seed_waiting_ticket(&db, Some(&cid), 1, "W001").await;

    let idemp_key = unique("idemp-call");
    let ans1 = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/queue/tickets/{tid}/call"),
        Some(json!({ "branch": B01 })),
        Some(&idemp_key),
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "expected 200; got {}",
        ans1.text
    );

    let count_before = fetch_notifications(&db).await.len();
    assert_eq!(count_before, 1, "the first call writes exactly one notice");

    let ans2 = call_route(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/queue/tickets/{tid}/call"),
        Some(json!({ "branch": B01 })),
        Some(&idemp_key),
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "expected 200; got {}",
        ans2.text
    );

    let rows_after = fetch_notifications(&db).await;
    assert_eq!(
        rows_after.len(),
        count_before,
        "the replay writes nothing; count before = {}, count after = {}",
        count_before,
        rows_after.len()
    );
    if count_before > 0 {
        assert_eq!(
            count_before, 1,
            "row 6 requires one row; got {}",
            count_before
        );
    }

    db.finish().await;
}
