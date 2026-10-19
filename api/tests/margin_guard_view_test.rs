//! Integration tests for #3402: `GET /api/v3/views/margin_guard` reads the
//! caller's organisation's `margin_guards` row under `RatePolicySave`.

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

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-000000000099";
const EXEC_USER_ID: &str = "00000000-0000-0000-0000-000000000002";
const OTHER_USER_ID: &str = "00000000-0000-0000-0000-000000000098";
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
        let schema = unique("mg_3402");
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
             VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Exec User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed exec user");

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
    match spec {
        "EXECUTIVE" => Some(Actor::new(
            EXEC_USER_ID,
            ORG,
            vec![ScopedRole::new(Role::Executive, None, from, None)],
            None,
        )),
        "MARKETING" => Some(Actor::new(
            "user-MARKETING",
            ORG,
            vec![ScopedRole::new(Role::Marketing, None, from, None)],
            None,
        )),
        _ => None,
    }
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

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

// 1. no margin_guards row -> all three null
#[actix_web::test]
async fn row1_no_margin_guards_row_returns_all_null() {
    let room = Room::start().await;

    let ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::GET,
        "/api/v3/views/margin_guard",
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert_eq!(
        ans.body["data"],
        json!({
            "maxSpreadShare": null,
            "setBy": null,
            "setAt": null,
        })
    );

    room.finish().await;
}

// 2. after marginGuardSet { maxSpreadShare: "0.3" } by an EXECUTIVE ->
// maxSpreadShare: "0.3000", setBy = that user's id, setAt set (RFC 3339)
#[actix_web::test]
async fn row2_after_margin_guard_set_returns_share_setter_and_rfc3339_timestamp() {
    let room = Room::start().await;

    let set_ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "0.3" }), &unique("mg_set_03"))),
    )
    .await;
    assert_eq!(
        set_ans.status,
        StatusCode::OK,
        "set must succeed: {}",
        set_ans.text
    );

    let ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::GET,
        "/api/v3/views/margin_guard",
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert_eq!(ans.body["data"]["maxSpreadShare"], json!("0.3000"));
    assert_eq!(ans.body["data"]["setBy"], json!(EXEC_USER_ID));
    let set_at = ans.body["data"]["setAt"]
        .as_str()
        .expect("setAt must be a string");
    DateTime::parse_from_rfc3339(set_at).expect("setAt must be valid RFC 3339");

    room.finish().await;
}

// 3. after a set to null (the guard switched off) ->
// maxSpreadShare: null, setBy/setAt the last setter's
#[actix_web::test]
async fn row3_after_set_to_null_returns_null_share_with_last_setter() {
    let room = Room::start().await;

    let set1 = call(
        &room,
        Some("EXECUTIVE"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "0.3" }), &unique("mg_set1"))),
    )
    .await;
    assert_eq!(
        set1.status,
        StatusCode::OK,
        "first set must succeed: {}",
        set1.text
    );

    let set2 = call(
        &room,
        Some("EXECUTIVE"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": null }), &unique("mg_set_null"))),
    )
    .await;
    assert_eq!(
        set2.status,
        StatusCode::OK,
        "null set must succeed: {}",
        set2.text
    );

    let ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::GET,
        "/api/v3/views/margin_guard",
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert_eq!(ans.body["data"]["maxSpreadShare"], Value::Null);
    assert_eq!(ans.body["data"]["setBy"], json!(EXEC_USER_ID));
    let set_at = ans.body["data"]["setAt"]
        .as_str()
        .expect("setAt must remain set");
    DateTime::parse_from_rfc3339(set_at).expect("setAt must be valid RFC 3339");

    room.finish().await;
}

// 4. another organisation's row -> never shown
#[actix_web::test]
async fn row4_another_organisation_row_never_shown() {
    let room = Room::start().await;
    let other_org = Uuid::parse_str(OTHER_ORG).unwrap();
    let other_user = Uuid::parse_str(OTHER_USER_ID).unwrap();

    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1, 'OTHER', 'Other Org', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(other_org)
    .execute(&room.pool)
    .await
    .expect("seed other org");

    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
         VALUES ($1, $2, 'Other User', 'ACTIVE', 1, now(), 'test') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(other_user)
    .bind(other_org)
    .execute(&room.pool)
    .await
    .expect("seed other user");

    sqlx::query(
        "INSERT INTO margin_guards (organization_id, max_spread_share, set_by, set_at) \
         VALUES ($1, 0.7500, $2, now())",
    )
    .bind(other_org)
    .bind(other_user)
    .execute(&room.pool)
    .await
    .expect("seed other org margin_guard");

    let ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::GET,
        "/api/v3/views/margin_guard",
        None,
    )
    .await;

    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert_eq!(
        ans.body["data"],
        json!({
            "maxSpreadShare": null,
            "setBy": null,
            "setAt": null,
        })
    );

    room.finish().await;
}

// 5. caller without RatePolicySave (e.g. MARKETING) -> refused
#[actix_web::test]
async fn row5_caller_without_rate_policy_save_is_refused() {
    let room = Room::start().await;

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::GET,
        "/api/v3/views/margin_guard",
        None,
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "MARKETING must be refused: {}",
        ans.text
    );
    assert_eq!(
        ans.body["error"]["code"],
        json!("roleNotAllowedHere"),
        "expected roleNotAllowedHere: {}",
        ans.text
    );

    room.finish().await;
}
