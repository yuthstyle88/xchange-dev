//! Integration tests for #2395: `marginGuardSet` (`PUT /api/margin-guard`)
//! must require an active `EXECUTIVE` role (`is_executive_now`), refusing
//! branch `RATE_MANAGER`, `MARKETING`, and expired `EXECUTIVE` + active
//! `RATE_MANAGER` with `400 roleNotAllowedHere`.

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
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
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
        let schema = unique("mg_2395");
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
             VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
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
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    match spec {
        "EXECUTIVE" => Some(Actor::new(
            "user-EXECUTIVE",
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
        "RATE_MANAGER" => Some(Actor::new(
            "user-RATE_MANAGER",
            ORG,
            vec![ScopedRole::new(
                Role::RateManager,
                Some(BranchId::new(MAIN)),
                from,
                None,
            )],
            None,
        )),
        "EXPIRED_EXECUTIVE_AND_RATE_MANAGER" => Some(Actor::new(
            "user-EXPIRED_EXEC_RM",
            ORG,
            vec![
                ScopedRole::new(
                    Role::Executive,
                    None,
                    from,
                    Some(from + chrono::Duration::seconds(1)),
                ),
                ScopedRole::new(Role::RateManager, Some(BranchId::new(MAIN)), from, None),
            ],
            None,
        )),
        _ => None,
    }
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

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
            .to_owned()
    }
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

// M1: RATE_MANAGER on MAIN calls PUT /api/margin-guard with {"maxSpreadShare": "1"}
// -> 400 roleNotAllowedHere; margin_guards has no row.
#[actix_web::test]
async fn m1_rate_manager_cannot_set_margin_guard() {
    let room = Room::start().await;

    let ans = call(
        &room,
        Some("RATE_MANAGER"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "1" }), &unique("m1_rm"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "M1: marginGuardSet must refuse branch RATE_MANAGER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "M1: expected roleNotAllowedHere, got: {}",
        ans.text
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM margin_guards")
        .fetch_one(&room.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "M1: margin_guards must remain empty");

    room.finish().await;
}

// M2: margin_guards has an existing row (0.5000); RATE_MANAGER on MAIN calls
// PUT /api/margin-guard with {"maxSpreadShare": null} -> 400 roleNotAllowedHere;
// row in margin_guards stays 0.5000.
#[actix_web::test]
async fn m2_rate_manager_cannot_disable_existing_margin_guard() {
    let room = Room::start().await;
    let org_id = Uuid::parse_str(ORG).unwrap();

    let seed_ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::PUT,
        "/api/margin-guard",
        Some((
            json!({ "maxSpreadShare": "0.5000" }),
            &unique("m2_exec_seed"),
        )),
    )
    .await;
    assert_eq!(
        seed_ans.status,
        StatusCode::OK,
        "seed via EXECUTIVE must succeed"
    );

    let ans = call(
        &room,
        Some("RATE_MANAGER"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": null }), &unique("m2_rm_null"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "M2: marginGuardSet must refuse branch RATE_MANAGER disabling guard with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "M2: expected roleNotAllowedHere, got: {}",
        ans.text
    );

    let share: Option<Decimal> =
        sqlx::query_scalar("SELECT max_spread_share FROM margin_guards WHERE organization_id = $1")
            .bind(org_id)
            .fetch_one(&room.pool)
            .await
            .unwrap();
    assert_eq!(
        share,
        Some(Decimal::new(5000, 4)),
        "M2: row in margin_guards must stay 0.5000"
    );

    room.finish().await;
}

// M3: EXECUTIVE (org-wide) calls PUT /api/margin-guard with {"maxSpreadShare": "0.5000"}
// -> 200, row 0.5000 written.
#[actix_web::test]
async fn m3_executive_can_set_margin_guard() {
    let room = Room::start().await;
    let org_id = Uuid::parse_str(ORG).unwrap();

    let ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "0.5000" }), &unique("m3_exec"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "M3: marginGuardSet must accept EXECUTIVE: {}",
        ans.text
    );

    let share: Option<Decimal> =
        sqlx::query_scalar("SELECT max_spread_share FROM margin_guards WHERE organization_id = $1")
            .bind(org_id)
            .fetch_one(&room.pool)
            .await
            .unwrap();
    assert_eq!(
        share,
        Some(Decimal::new(5000, 4)),
        "M3: row 0.5000 must be written to margin_guards"
    );

    room.finish().await;
}

// M4: MARKETING calls PUT /api/margin-guard with {"maxSpreadShare": "0.5000"}
// -> 400 roleNotAllowedHere.
#[actix_web::test]
async fn m4_marketing_cannot_set_margin_guard() {
    let room = Room::start().await;

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "0.5000" }), &unique("m4_mktg"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "M4: marginGuardSet must refuse MARKETING with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "M4: expected roleNotAllowedHere, got: {}",
        ans.text
    );

    room.finish().await;
}

// M5: EXECUTIVE whose role row has valid_to in the past + RATE_MANAGER on MAIN
// calls PUT /api/margin-guard with {"maxSpreadShare": "1"} -> 400 roleNotAllowedHere;
// margin_guards has no row.
#[actix_web::test]
async fn m5_expired_executive_with_active_rate_manager_cannot_set_margin_guard() {
    let room = Room::start().await;

    let ans = call(
        &room,
        Some("EXPIRED_EXECUTIVE_AND_RATE_MANAGER"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "1" }), &unique("m5_exp_exec_rm"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "M5: marginGuardSet must refuse expired EXECUTIVE + active RATE_MANAGER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "M5: expected roleNotAllowedHere, got: {}",
        ans.text
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM margin_guards")
        .fetch_one(&room.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "M5: margin_guards must remain empty");

    room.finish().await;
}
