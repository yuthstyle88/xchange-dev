//! Specification tests for #2486: queue ticket commands (`call`, `recall`, `skip`, `start`,
//! `finish`, `cancel`) must parse `{id}` as a UUID and refuse a non-UUID `{id}` with
//! `400 validationFailed` on field `id` before touching the database.

mod common;

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

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

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
        let schema = unique("qu2486test");
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn post(db: &TestDb, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "TELLER@XC-B01"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("k-2486")))
        .set_payload(body.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

fn assert_invalid_id(ans: &Answer, what: &str) {
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{what}: expected 400 BAD_REQUEST; got {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(
        ans.body["error"]["code"], "validationFailed",
        "{what}: expected error.code == validationFailed; got {}",
        ans.text
    );
    assert_eq!(
        ans.body["error"]["field"], "id",
        "{what}: expected error.field == id; got {}",
        ans.text
    );
    assert!(
        ans.body["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "{what}: expected non-empty error.message; got {}",
        ans.text
    );
}

/// Row 1: `POST /api/queue/tickets/abc/call` (non-UUID id) -> 400 `validationFailed`, field `id`; no DB write.
#[actix_web::test]
async fn call_with_non_uuid_id_is_refused_400_validation_failed_on_id_and_writes_nothing() {
    let db = TestDb::start().await;
    let w1 = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now(), 'key-2486-w1')",
    )
    .bind(w1)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("seed waiting ticket");

    let ans = post(&db, "/api/queue/tickets/abc/call", json!({ "branch": B01 })).await;
    assert_invalid_id(&ans, "POST /api/queue/tickets/abc/call");

    let status: String = sqlx::query_scalar("SELECT status FROM queue_tickets WHERE id = $1")
        .bind(w1)
        .fetch_one(&db.pool)
        .await
        .expect("read seeded ticket");
    assert_eq!(status, "WAITING", "seeded ticket must remain WAITING");

    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM queue_events")
        .fetch_one(&db.pool)
        .await
        .expect("count queue_events");
    assert_eq!(events, 0, "no queue_events row may be written");

    db.finish().await;
}

/// Row 2: Same for `recall`, `skip`, `start`, `finish` -> 400 `validationFailed`, field `id`.
#[actix_web::test]
async fn recall_skip_start_finish_with_non_uuid_id_are_refused_400_validation_failed_on_id() {
    let db = TestDb::start().await;

    for verb in ["recall", "skip", "start", "finish"] {
        let uri = format!("/api/queue/tickets/abc/{verb}");
        let ans = post(&db, &uri, json!({ "branch": B01 })).await;
        assert_invalid_id(&ans, &format!("POST {uri}"));
    }

    db.finish().await;
}

/// Row 3: `POST /api/queue/tickets/abc/cancel` with a reason -> 400 `validationFailed`, field `id`.
#[actix_web::test]
async fn cancel_with_non_uuid_id_and_reason_is_refused_400_validation_failed_on_id() {
    let db = TestDb::start().await;

    let ans = post(
        &db,
        "/api/queue/tickets/abc/cancel",
        json!({ "branch": B01, "reason": "customerLeft" }),
    )
    .await;
    assert_invalid_id(&ans, "POST /api/queue/tickets/abc/cancel");

    db.finish().await;
}

/// Row 4: A well-formed UUID that names no ticket -> unchanged from today (`404 notFound`, never 500).
#[actix_web::test]
async fn unknown_well_formed_uuid_answers_404_not_found() {
    let db = TestDb::start().await;
    let missing = Uuid::new_v4();

    for verb in ["call", "recall", "skip", "start", "finish"] {
        let uri = format!("/api/queue/tickets/{missing}/{verb}");
        let ans = post(&db, &uri, json!({ "branch": B01 })).await;
        assert_eq!(
            ans.status,
            StatusCode::NOT_FOUND,
            "POST {uri}: expected 404 NOT_FOUND; got {} {}",
            ans.status,
            ans.text
        );
        assert_eq!(
            ans.body["error"]["code"], "notFound",
            "POST {uri}: expected error.code == notFound; got {}",
            ans.text
        );
    }

    let cancel_uri = format!("/api/queue/tickets/{missing}/cancel");
    let ans = post(
        &db,
        &cancel_uri,
        json!({ "branch": B01, "reason": "customerLeft" }),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "POST {cancel_uri}: expected 404 NOT_FOUND; got {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(
        ans.body["error"]["code"], "notFound",
        "POST {cancel_uri}: expected error.code == notFound; got {}",
        ans.text
    );

    db.finish().await;
}

/// Row 5 & §9 edge case: A valid ticket (including an upper-case UUID accepted by `Uuid::parse_str`)
/// succeeds and passes the hyphenated lower-case form on.
#[actix_web::test]
async fn valid_ticket_including_uppercase_uuid_succeeds() {
    let db = TestDb::start().await;
    let w1 = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now(), 'key-2486-valid')",
    )
    .bind(w1)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("seed waiting ticket");

    let upper = w1.to_string().to_uppercase();
    let ans = post(
        &db,
        &format!("/api/queue/tickets/{upper}/call"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "call with uppercase UUID should succeed; got {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.body["id"], w1.to_string());
    assert_eq!(ans.body["status"], "CALLED");

    let ans = post(
        &db,
        &format!("/api/queue/tickets/{upper}/cancel"),
        json!({ "branch": B01, "reason": "customerLeft" }),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "cancel with uppercase UUID should succeed; got {} {}",
        ans.status,
        ans.text
    );
    assert_eq!(ans.body["id"], w1.to_string());
    assert_eq!(ans.body["status"], "CANCELLED");

    db.finish().await;
}
