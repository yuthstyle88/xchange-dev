//! #2029: Every v3 5xx logs the internal detail server-side and returns the generic
//! message `เซิร์ฟเวอร์ขัดข้อง` to the client without leaking Postgres constraint or table names.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::body::to_bytes;
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, ResponseError, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TEST_USER: &str = "00000000-0000-0000-0000-000000000002";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("v3_err_2029");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
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
            .expect("connect to test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
             VALUES ($1, $2, 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::parse_str(TEST_USER).unwrap())
        .bind(Uuid::parse_str(ORG).unwrap())
        .execute(&pool)
        .await
        .expect("seed test user");

        common::install_test_pii_keys();

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

fn user_of(spec: &str) -> Option<Actor> {
    if spec.is_empty() {
        return None;
    }
    let from = DateTime::<Utc>::UNIX_EPOCH;
    let role = match spec {
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        _ => return None,
    };
    let branch = if role == Role::Teller {
        Some(BranchId::new(MAIN))
    } else {
        None
    };
    let user_id = format!("user-{}", role.as_str());
    Some(Actor::new(
        &user_id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user.as_ref().and_then(|u| {
            if u.roles.iter().any(|r| r.role != Role::Customer) {
                Some(u.clone())
            } else {
                None
            }
        });
        Box::pin(async move { Callers { user, staff_user } })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(
    room: &Room,
    who: Option<&str>,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let guard = room.guard.clone();
    let app = atest::init_service(
        App::new()
            .app_data(web::Data::from(guard))
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"));

    if let Some(w) = who {
        req = req.insert_header((AS, w));
    }
    if let Some((b, key)) = body {
        req = req
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(b.to_string());
    }

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

// Row 1: V3Error::new("database", None, "...", 500).error_response()
#[actix_web::test]
async fn row_1_500_error_response_returns_generic_message_without_table_or_constraint_names() {
    let resp = V3Error::new(
        "database",
        None,
        "new row for relation \"campaign_versions\" violates check constraint \"campaign_versions_check\"",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(resp.into_body()).await.expect("read body");
    let text = String::from_utf8_lossy(&bytes);
    let body: Value = serde_json::from_str(&text).expect("valid json");

    assert_eq!(body["error"]["code"], "database");
    assert_eq!(body["error"]["message"], "เซิร์ฟเวอร์ขัดข้อง");
    assert!(
        !text.contains("campaign_versions"),
        "500 response body must not leak table name: {text}"
    );
    assert!(
        !text.contains("constraint"),
        "500 response body must not leak constraint text: {text}"
    );
}

// Row 2: A real constraint violation through a route (`campaignSubmit` hitting `approval_requests_type_idempotency_key_key`)
#[actix_web::test]
async fn row_2_real_constraint_violation_through_route_hides_table_and_constraint_names() {
    let room = Room::start().await;

    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str(TEST_USER).unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'Submit Constraint Campaign', NULL, $3, now())",
    )
    .bind(campaign_id)
    .bind(org_id)
    .bind(user_id)
    .execute(&room.pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 1, 'SIMULATED', now(), now() + interval '10 days', 1000.00, 'RATE', 1, $4, now() - interval '1 hour', now() - interval '1 hour')",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(user_id)
    .execute(&room.pool)
    .await
    .expect("seed campaign_version");

    sqlx::query(
        "INSERT INTO campaign_simulations ( \
             id, campaign_version_id, input, result, cost_source, simulated_by, simulated_at \
         ) VALUES ($1, $2, '{}'::jsonb, '{}'::jsonb, 'LEDGER', $3, now())",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .bind(user_id)
    .execute(&room.pool)
    .await
    .expect("seed campaign_simulation");

    sqlx::query(
        "INSERT INTO approval_requests ( \
             id, organization_id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key \
         ) VALUES ($1, $2, 'CAMPAIGN_VERSION', NULL, $3, 'prior-hash', '1', '{}'::jsonb, $4, now(), 'REJECTED', $5)",
    )
    .bind(Uuid::new_v4())
    .bind(org_id)
    .bind(version_id.to_string())
    .bind(user_id.to_string())
    .bind(format!("campaign_version_submit_{version_id}"))
    .execute(&room.pool)
    .await
    .expect("seed prior approval_request");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &format!("/api/campaigns/{campaign_id}/submit"),
        Some((json!({}), &unique("submit_campaign_constraint"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "expected 500 from DB constraint violation: {}",
        ans.text
    );
    assert_eq!(ans.body["error"]["code"], "database");
    assert_eq!(ans.body["error"]["message"], "เซิร์ฟเวอร์ขัดข้อง");
    assert!(
        !ans.text.contains("approval_requests"),
        "500 route response must not contain table name `approval_requests`: {}",
        ans.text
    );
    assert!(
        !ans.text.contains("constraint"),
        "500 route response must not contain `constraint`: {}",
        ans.text
    );

    room.finish().await;
}

// Row 3: A 4xx keeps its message unchanged
#[actix_web::test]
async fn row_3_4xx_error_response_keeps_user_facing_message() {
    let resp = V3Error::new(
        "validationFailed",
        Some("x"),
        "ต้องระบุ x",
        StatusCode::BAD_REQUEST,
    )
    .error_response();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(resp.into_body()).await.expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("valid json");

    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "x");
    assert_eq!(body["error"]["message"], "ต้องระบุ x");
}

// Row 4: A 5xx with a field keeps the field and uses the generic message
#[actix_web::test]
async fn row_4_5xx_with_field_keeps_field_and_uses_generic_message() {
    let resp = V3Error::new(
        "database",
        Some("startsAt"),
        "new row for relation \"campaign_versions\" violates check constraint \"campaign_versions_check\"",
        StatusCode::BAD_GATEWAY,
    )
    .error_response();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let bytes = to_bytes(resp.into_body()).await.expect("read body");
    let text = String::from_utf8_lossy(&bytes);
    let body: Value = serde_json::from_str(&text).expect("valid json");

    assert_eq!(body["error"]["code"], "database");
    assert_eq!(body["error"]["field"], "startsAt");
    assert_eq!(body["error"]["message"], "เซิร์ฟเวอร์ขัดข้อง");
    assert!(
        !text.contains("campaign_versions") && !text.contains("constraint"),
        "5xx with field must not leak internal detail: {text}"
    );
}
