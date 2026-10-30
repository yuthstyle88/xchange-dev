//! Tests for #5685: queue_board adds tellerName to each CALLED/SERVING row.
//! Rows B-1 through B-6 from issue #5710.

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
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::pages::{Callers, SessionReader};

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
        let schema = xc_testdb::unique_schema_name("qb_teller");
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
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid)",
        )
        .bind(B01)
        .bind(CALLER_TELLER_ID)
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
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
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

/// B-1: SERVING ticket at a teller whose `users.name` is "Somchai Jaidee".
/// `called[i].tellerName` is "Somchai Jaidee", and `tellerId` is still that teller's id.
#[actix_web::test]
async fn test_b1_serving_ticket_at_teller_with_name() {
    let db = TestDb::start().await;
    let teller_uuid = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'Somchai Jaidee', 'ACTIVE', 'test')",
    )
    .bind(teller_uuid)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("insert teller");

    let ticket_uuid = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, served_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'SERVING', $4, now() - interval '5m', now() - interval '2m', now() - interval '1m', 'key-b1')",
    )
    .bind(ticket_uuid)
    .bind(ORG)
    .bind(B01)
    .bind(teller_uuid)
    .execute(&db.pool)
    .await
    .expect("insert serving ticket");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");
    let ans = call(&db, &uri).await;
    assert_eq!(ans.status, StatusCode::OK);

    let called = ans.body["data"]["called"].as_array().expect("called array");
    let row = called
        .iter()
        .find(|r| r["displayNumber"] == "W001")
        .expect("ticket W001 in called");
    assert_eq!(row["tellerName"], "Somchai Jaidee");
    assert_eq!(row["tellerId"], teller_uuid.to_string());
    assert_eq!(row["status"], "SERVING");

    db.finish().await;
}

/// B-2: CALLED ticket at a teller with `users.name` "Nok" while another teller serves another ticket.
/// Each row carries its own teller's name; names are not swapped between rows.
#[actix_web::test]
async fn test_b2_called_and_serving_tickets_carry_own_teller_names() {
    let db = TestDb::start().await;
    let teller_nok = Uuid::new_v4();
    let teller_somchai = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $3::uuid, 'Nok', 'ACTIVE', 'test'), ($2, $3::uuid, 'Somchai', 'ACTIVE', 'test')",
    )
    .bind(teller_nok)
    .bind(teller_somchai)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("insert tellers");

    let t_called = Uuid::new_v4();
    let t_serving = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, idempotency_key) \
         VALUES ($1, $3::uuid, $4, current_date, 'WALK_IN', 1, 'W001', 'CALLED', $2, now() - interval '5m', now() - interval '2m', 'key-b2-called')",
    )
    .bind(t_called)
    .bind(teller_nok)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert called ticket");

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, served_at, idempotency_key) \
         VALUES ($1, $3::uuid, $4, current_date, 'WALK_IN', 2, 'W002', 'SERVING', $2, now() - interval '4m', now() - interval '1m', now(), 'key-b2-serving')",
    )
    .bind(t_serving)
    .bind(teller_somchai)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert serving ticket");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");
    let ans = call(&db, &uri).await;
    assert_eq!(ans.status, StatusCode::OK);

    let called = ans.body["data"]["called"].as_array().expect("called array");
    let row_called = called
        .iter()
        .find(|r| r["displayNumber"] == "W001")
        .expect("ticket W001 in called");
    let row_serving = called
        .iter()
        .find(|r| r["displayNumber"] == "W002")
        .expect("ticket W002 in called");

    assert_eq!(row_called["tellerName"], "Nok");
    assert_eq!(row_called["tellerId"], teller_nok.to_string());
    assert_eq!(row_called["status"], "CALLED");

    assert_eq!(row_serving["tellerName"], "Somchai");
    assert_eq!(row_serving["tellerId"], teller_somchai.to_string());
    assert_eq!(row_serving["status"], "SERVING");

    assert_ne!(row_called["tellerName"], row_serving["tellerName"]);
    assert_ne!(row_called["tellerId"], row_serving["tellerId"]);

    db.finish().await;
}

/// B-3: A CALLED or SERVING row with `counter_teller_id` NULL.
/// `tellerName` is null; the row is still listed.
#[actix_web::test]
async fn test_b3_called_or_serving_row_with_null_counter_teller_id() {
    let db = TestDb::start().await;

    let t_called = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'CALLED', NULL, now() - interval '5m', now() - interval '2m', 'key-b3')",
    )
    .bind(t_called)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert called ticket with null counter_teller_id");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");
    let ans = call(&db, &uri).await;
    assert_eq!(ans.status, StatusCode::OK);

    let called = ans.body["data"]["called"].as_array().expect("called array");
    assert_eq!(called.len(), 1, "the row is still listed");
    let row = &called[0];
    assert_eq!(row["displayNumber"], "W001");
    assert!(row["tellerId"].is_null(), "tellerId is null");
    assert!(
        row.as_object()
            .is_some_and(|o| o.contains_key("tellerName")),
        "the row carries the tellerName key"
    );
    assert!(row["tellerName"].is_null(), "tellerName is null");

    db.finish().await;
}

/// B-4: The teller's `users.name` is '' or only spaces.
/// `called[i].tellerName` is null.
#[actix_web::test]
async fn test_b4_teller_users_name_empty_or_spaces_is_null() {
    let db = TestDb::start().await;

    let teller_empty = Uuid::new_v4();
    let teller_spaces = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $3::uuid, '', 'ACTIVE', 'test'), ($2, $3::uuid, '   ', 'ACTIVE', 'test')",
    )
    .bind(teller_empty)
    .bind(teller_spaces)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("insert tellers with empty and space names");

    let t1 = Uuid::new_v4();
    let t2 = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, counter_teller_id, issued_at, called_at, idempotency_key) \
         VALUES \
           ($1, $3::uuid, $4, current_date, 'WALK_IN', 1, 'W001', 'CALLED', $5, now() - interval '5m', now() - interval '2m', 'key-b4-1'), \
           ($2, $3::uuid, $4, current_date, 'WALK_IN', 2, 'W002', 'SERVING', $6, now() - interval '4m', now() - interval '1m', 'key-b4-2')",
    )
    .bind(t1)
    .bind(t2)
    .bind(ORG)
    .bind(B01)
    .bind(teller_empty)
    .bind(teller_spaces)
    .execute(&db.pool)
    .await
    .expect("insert tickets");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");
    let ans = call(&db, &uri).await;
    assert_eq!(ans.status, StatusCode::OK);

    let called = ans.body["data"]["called"].as_array().expect("called array");
    let row1 = called
        .iter()
        .find(|r| r["displayNumber"] == "W001")
        .expect("W001 in called");
    let row2 = called
        .iter()
        .find(|r| r["displayNumber"] == "W002")
        .expect("W002 in called");

    assert_eq!(row1["tellerId"], teller_empty.to_string());
    assert!(
        row1.as_object()
            .is_some_and(|o| o.contains_key("tellerName")),
        "row1 carries the tellerName key"
    );
    assert!(
        row1["tellerName"].is_null(),
        "empty name yields null tellerName"
    );

    assert_eq!(row2["tellerId"], teller_spaces.to_string());
    assert!(
        row2.as_object()
            .is_some_and(|o| o.contains_key("tellerName")),
        "row2 carries the tellerName key"
    );
    assert!(
        row2["tellerName"].is_null(),
        "whitespace name yields null tellerName"
    );

    db.finish().await;
}

/// B-5: The generated type: committed `queue_board.ts` == the view's export, and it contains `tellerName: string | null`.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_b5_generated_type() {
    let view = VIEWS
        .iter()
        .find(|v| v.name == "queue_board")
        .expect("queue_board view entry");
    let scratch = Scratch(std::env::temp_dir().join(xc_testdb::unique_schema_name("qb_ts")));
    std::fs::create_dir_all(&scratch.0).expect("create scratch dir");
    (view.export_types)(&scratch.0).expect("export queue_board types");

    let ts = std::fs::read_to_string(scratch.0.join("queue_board.ts"))
        .expect("read generated queue_board.ts");
    assert!(
        ts.contains("tellerName: string | null"),
        "queue_board.ts must contain `tellerName: string | null`: {ts}"
    );

    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/queue_board.ts");
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("read committed queue_board.ts: {e}"));
    assert_eq!(
        committed, ts,
        "committed queue_board.ts must match exported queue_board.ts byte for byte"
    );
}

/// B-6: WAITING rows and `myCounter`: unchanged shape, with no `tellerName` on waiting rows.
#[actix_web::test]
async fn test_b6_waiting_rows_and_my_counter_unchanged_shape() {
    let db = TestDb::start().await;

    let t_waiting = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now() - interval '5m', 'key-b6')",
    )
    .bind(t_waiting)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert waiting ticket");

    let uri = format!("/api/v3/views/queue_board?branch={B01}");
    let ans = call(&db, &uri).await;
    assert_eq!(ans.status, StatusCode::OK);

    let data = &ans.body["data"];
    let waiting = data["waiting"].as_array().expect("waiting array");
    assert_eq!(waiting.len(), 1);
    let w = &waiting[0];
    assert_eq!(w["displayNumber"], "W001");
    assert_eq!(w["serviceType"], "WALK_IN");
    assert_eq!(w["sequence"], 1);
    assert!(
        w.get("tellerName").is_none(),
        "waiting row must not have tellerName"
    );
    assert!(
        w.get("tellerId").is_none(),
        "waiting row must not have tellerId"
    );

    let my_counter = &data["myCounter"];
    assert!(my_counter.is_object(), "myCounter is object");
    assert_eq!(my_counter["status"], "ACTIVE");
    assert!(
        my_counter.get("tellerName").is_none(),
        "myCounter must not have tellerName"
    );

    db.finish().await;
}
