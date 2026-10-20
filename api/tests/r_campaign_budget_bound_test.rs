//! O-G1-2 R-ticket (#3637, parent #2054):
//! `budgetThb` on `POST /api/campaigns` (`campaignCreate`) and
//! `PUT /api/campaigns/{id}/draft` (`campaignVersionSave`) must be parsed through
//! `v3::input::decimal("budgetThb", &input.budget_thb, Bound::numeric(20, 2, Sign::NonNegative))`.

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
        let schema = unique("r_campaign_budget");
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
    let role = match spec {
        "MARKETING" => Role::Marketing,
        _ => return None,
    };
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    let user_id = format!("user-{}", role.as_str());
    Some(Actor::new(
        &user_id,
        ORG,
        vec![ScopedRole::new(role, None, from, None)],
        None,
    ))
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
        let staff_user = user.clone();
        Box::pin(async move { Callers { user, staff_user } })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
    }

    fn field(&self) -> &str {
        self.body["error"]["field"]
            .as_str()
            .or_else(|| self.body["field"].as_str())
            .unwrap_or("")
    }
}

async fn call(room: &Room, method: Method, uri: &str, payload: Value) -> Answer {
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

    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "MARKETING"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique("idem")))
        .set_payload(payload.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    Answer { status, text, body }
}

async fn seed_draft_campaign(pool: &PgPool) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'Seeded Draft Campaign', NULL, $3, now())",
    )
    .bind(campaign_id)
    .bind(org_id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 1, 'DRAFT', now() - interval '1 hour', now() + interval '10 days', \
                   10000.00, 'RATE', 1, NULL, $4, now(), now())",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("seed version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(pool)
        .await
        .expect("update current_version_id");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
         VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed budget");

    sqlx::query(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) \
         VALUES ($1, 'MAIN')",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed campaign_branches");

    (campaign_id, version_id)
}

async fn create_campaign(room: &Room, budget_thb: &str) -> Answer {
    call(
        room,
        Method::POST,
        "/api/campaigns",
        json!({
            "name": "Bound Test Campaign",
            "startsAt": "2026-11-01T00:00:00Z",
            "endsAt": "2026-12-01T00:00:00Z",
            "budgetThb": budget_thb,
            "stackGroup": "RATE",
            "priority": 1,
            "branches": ["MAIN"]
        }),
    )
    .await
}

async fn save_draft(room: &Room, campaign_id: Uuid, budget_thb: &str) -> Answer {
    let uri = format!("/api/campaigns/{campaign_id}/draft");
    call(
        room,
        Method::PUT,
        &uri,
        json!({
            "startsAt": "2026-11-01T00:00:00Z",
            "endsAt": "2026-12-01T00:00:00Z",
            "budgetThb": budget_thb,
            "stackGroup": "RATE",
            "priority": 1,
            "branches": ["MAIN"]
        }),
    )
    .await
}

fn assert_refused_budget(ans: &Answer, route: &str, input: &str) {
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "{route} ({input}): expected 400, got {}: {}",
        ans.status,
        ans.text
    );
    assert_eq!(
        ans.code(),
        "validationFailed",
        "{route} ({input}): expected validationFailed, got: {}",
        ans.text
    );
    assert_eq!(
        ans.field(),
        "budgetThb",
        "{route} ({input}): expected field budgetThb, got: {}",
        ans.text
    );
}

async fn assert_both_routes_refuse_and_preserve_rows(budget_thb: &str) {
    let room = Room::start().await;

    let create_ans = create_campaign(&room, budget_thb).await;
    let campaigns_after_create: i64 = sqlx::query_scalar("SELECT count(*) FROM campaigns")
        .fetch_one(&room.pool)
        .await
        .expect("count campaigns");
    let versions_after_create: i64 = sqlx::query_scalar("SELECT count(*) FROM campaign_versions")
        .fetch_one(&room.pool)
        .await
        .expect("count campaign_versions");

    let (campaign_id, version_id) = seed_draft_campaign(&room.pool).await;
    let save_ans = save_draft(&room, campaign_id, budget_thb).await;
    let stored_budget: String =
        sqlx::query_scalar("SELECT budget_thb::text FROM campaign_versions WHERE id = $1")
            .bind(version_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch seeded version budget_thb");

    room.finish().await;

    assert_refused_budget(&create_ans, "POST /api/campaigns", budget_thb);
    assert_eq!(
        campaigns_after_create, 0,
        "POST /api/campaigns ({budget_thb}) must not write campaigns row"
    );
    assert_eq!(
        versions_after_create, 0,
        "POST /api/campaigns ({budget_thb}) must not write campaign_versions row"
    );

    assert_refused_budget(&save_ans, "PUT /api/campaigns/{id}/draft", budget_thb);
    assert_eq!(
        stored_budget, "10000.00",
        "PUT /api/campaigns/{{id}}/draft ({budget_thb}) must not change existing budget_thb"
    );
}

async fn assert_both_routes_accept_and_store(budget_thb: &str, expected_stored: &str) {
    let room = Room::start().await;

    let create_ans = create_campaign(&room, budget_thb).await;
    assert_eq!(
        create_ans.status,
        StatusCode::OK,
        "POST /api/campaigns ({budget_thb}) failed: {}",
        create_ans.text
    );
    let created_id = Uuid::parse_str(
        create_ans.body["id"]
            .as_str()
            .expect("created campaign id string"),
    )
    .expect("created campaign uuid");
    let created_budget: String = sqlx::query_scalar(
        "SELECT budget_thb::text FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(created_id)
    .fetch_one(&room.pool)
    .await
    .expect("fetch created campaign_versions.budget_thb");
    assert_eq!(
        created_budget, expected_stored,
        "POST /api/campaigns ({budget_thb}) stored budget_thb mismatch"
    );

    let (draft_campaign_id, draft_version_id) = seed_draft_campaign(&room.pool).await;
    let save_ans = save_draft(&room, draft_campaign_id, budget_thb).await;
    assert_eq!(
        save_ans.status,
        StatusCode::OK,
        "PUT /api/campaigns/{{id}}/draft ({budget_thb}) failed: {}",
        save_ans.text
    );
    let saved_budget: String =
        sqlx::query_scalar("SELECT budget_thb::text FROM campaign_versions WHERE id = $1")
            .bind(draft_version_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch updated campaign_versions.budget_thb");
    assert_eq!(
        saved_budget, expected_stored,
        "PUT /api/campaigns/{{id}}/draft ({budget_thb}) stored budget_thb mismatch"
    );

    room.finish().await;
}

// §8 Row 1: "1000000000000000000" (10^18, overflows NUMERIC(20, 2)) -> 400 validationFailed
#[actix_web::test]
async fn row_1_ten_to_the_eighteen_is_refused_with_400_and_nothing_is_written() {
    assert_both_routes_refuse_and_preserve_rows("1000000000000000000").await;
}

// §8 Row 2: "100.005" (3 decimal places) -> 400 validationFailed
#[actix_web::test]
async fn row_2_three_decimal_places_is_refused_with_400_and_not_silently_rounded() {
    assert_both_routes_refuse_and_preserve_rows("100.005").await;
}

// §8 Row 3: "-1" -> 400 validationFailed
#[actix_web::test]
async fn row_3_negative_budget_is_refused_with_400() {
    assert_both_routes_refuse_and_preserve_rows("-1").await;
}

// §8 Row 4: "abc" -> 400 validationFailed
#[actix_web::test]
async fn row_4_non_numeric_budget_is_refused_with_400() {
    assert_both_routes_refuse_and_preserve_rows("abc").await;
}

// §8 Row 5: "0" -> accepted (200); stored 0.00 in campaign_versions.budget_thb
#[actix_web::test]
async fn row_5_zero_budget_is_accepted_and_stored_as_zero() {
    assert_both_routes_accept_and_store("0", "0.00").await;
}

// §8 Row 6: "999999999999999999.99" (max NUMERIC(20, 2)) -> accepted (200)
#[actix_web::test]
async fn row_6_max_numeric_20_2_budget_is_accepted_and_stored() {
    assert_both_routes_accept_and_store("999999999999999999.99", "999999999999999999.99").await;
}

// §8 Row 7: "1500.50" -> accepted (200); stored 1500.50 in campaign_versions.budget_thb
#[actix_web::test]
async fn row_7_ordinary_budget_is_accepted_and_stored() {
    assert_both_routes_accept_and_store("1500.50", "1500.50").await;
}

// §8 Row 8: " 100" (leading space) -> 400 validationFailed
#[actix_web::test]
async fn row_8_leading_space_budget_is_refused_with_400() {
    assert_both_routes_refuse_and_preserve_rows(" 100").await;
}
