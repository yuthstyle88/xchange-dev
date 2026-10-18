//! Validation tests for `campaignSimulate` (`POST /api/campaigns/{id}/simulate`, issue #1989).

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
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

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
        let schema = unique("sim_val_1989");
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

impl Answer {
    fn code(&self) -> String {
        self.body["error"]["code"]
            .as_str()
            .or_else(|| self.body["code"].as_str())
            .unwrap_or("")
            .to_owned()
    }

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
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

async fn seed_campaign(pool: &PgPool, status: &str) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let v_hash = if status == "DRAFT" {
        None
    } else {
        Some("v1_hash")
    };

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'Test Campaign', NULL, $3, now())",
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
         ) VALUES ($1, $2, $3, 1, $4, now() - interval '1 hour', now() + interval '10 days', \
                   10000.00, 'RATE', 1, $5, $6, now(), now())",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(status)
    .bind(v_hash)
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

async fn seed_published_usd_rate(pool: &PgPool) {
    let pub_request = Uuid::new_v4();
    let publication = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1, organization_id, 'RATE_PUBLICATION', id, $2, 'pub_hash', \
             '1', '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = 'MAIN'",
    )
    .bind(pub_request)
    .bind(publication.to_string())
    .bind(unique("pub"))
    .execute(pool)
    .await
    .expect("seed approval_request for publication");

    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
         SELECT $1, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
             now(), $2, NULL, 'test' \
         FROM branches b WHERE b.id = 'MAIN'",
    )
    .bind(publication)
    .bind(pub_request)
    .execute(pool)
    .await
    .expect("seed rate_publications");

    let snapshot = json!({
        "currency": "USD",
        "buy_mode": "MANUAL",
        "sell_mode": "MANUAL",
        "reason": "seed for campaign_simulate_validation_test",
    });

    sqlx::query(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT $1, organization_id, $2, 'USD', 'ALL', 1, 33.50000000, 34.00000000, $3 \
         FROM rate_publications WHERE id = $2",
    )
    .bind(line_id)
    .bind(publication)
    .bind(snapshot)
    .execute(pool)
    .await
    .expect("seed published_rate_lines");

    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = 'MAIN'")
        .bind(publication)
        .execute(pool)
        .await
        .expect("update branch current_publication_id");
}

async fn simulation_count(pool: &PgPool, version_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM campaign_simulations WHERE campaign_version_id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("count campaign_simulations")
}

async fn version_status(pool: &PgPool, version_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM campaign_versions WHERE id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("fetch campaign_versions.status")
}

// Row 1: `scenarios: []` on a DRAFT version -> 400 validationFailed (field `scenarios`);
// 0 new campaign_simulations rows; version status still DRAFT.
#[actix_web::test]
async fn campaign_simulate_refuses_empty_scenarios_with_400_and_leaves_draft_untouched() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/simulate");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({ "scenarios": [] }), &unique("sim_empty"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "empty scenarios must answer 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "body: {}", ans.text);
    assert_eq!(ans.field(), Some("scenarios"), "body: {}", ans.text);
    assert_eq!(
        simulation_count(&room.pool, version_id).await,
        0,
        "no campaign_simulations row may be written when scenarios is empty"
    );
    assert_eq!(
        version_status(&room.pool, version_id).await,
        "DRAFT",
        "version status must stay DRAFT when scenarios is empty"
    );

    room.finish().await;
}

// Row 2: one scenario with currency `"XYZ"` -> 400 validationFailed (field `currency`);
// 0 new rows; status unchanged.
#[actix_web::test]
async fn campaign_simulate_refuses_unknown_currency_with_400_and_leaves_draft_untouched() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/simulate");
    let payload = json!({
        "scenarios": [{
            "branchId": "MAIN",
            "currency": "XYZ",
            "amount": "100.00"
        }]
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("sim_bad_curr"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "unknown currency XYZ must answer 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "body: {}", ans.text);
    assert_eq!(ans.field(), Some("currency"), "body: {}", ans.text);
    assert_eq!(
        simulation_count(&room.pool, version_id).await,
        0,
        "no campaign_simulations row may be written on bad currency"
    );
    assert_eq!(
        version_status(&room.pool, version_id).await,
        "DRAFT",
        "version status must stay DRAFT on bad currency"
    );

    room.finish().await;
}

// Row 3: two scenarios, the second has currency `"XYZ"` -> 400 validationFailed (field `currency`);
// 0 rows (the first scenario does not slip through); status unchanged.
#[actix_web::test]
async fn campaign_simulate_refuses_second_scenario_with_unknown_currency_and_writes_zero_rows() {
    let room = Room::start().await;
    seed_published_usd_rate(&room.pool).await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/simulate");
    let payload = json!({
        "scenarios": [
            {
                "branchId": "MAIN",
                "currency": "USD",
                "amount": "100.00"
            },
            {
                "branchId": "MAIN",
                "currency": "XYZ",
                "amount": "50.00"
            }
        ]
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("sim_second_bad_curr"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "second scenario with unknown currency XYZ must answer 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "body: {}", ans.text);
    assert_eq!(ans.field(), Some("currency"), "body: {}", ans.text);
    assert_eq!(
        simulation_count(&room.pool, version_id).await,
        0,
        "first scenario must not slip through when second scenario has invalid currency"
    );
    assert_eq!(
        version_status(&room.pool, version_id).await,
        "DRAFT",
        "version status must stay DRAFT"
    );

    room.finish().await;
}

// Row 4: amount `"abc"` -> 400 validationFailed, field `amount` (kept).
#[actix_web::test]
async fn campaign_simulate_refuses_invalid_amount_with_400_validation_failed() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/simulate");
    let payload = json!({
        "scenarios": [{
            "branchId": "MAIN",
            "currency": "USD",
            "amount": "abc"
        }]
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("sim_bad_amt"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "invalid amount must answer 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "body: {}", ans.text);
    assert_eq!(ans.field(), Some("amount"), "body: {}", ans.text);
    assert_eq!(simulation_count(&room.pool, version_id).await, 0);
    assert_eq!(version_status(&room.pool, version_id).await, "DRAFT");

    room.finish().await;
}

// Row 5: one valid scenario (THB->USD on a published branch) -> 200; 1 row; DRAFT -> SIMULATED (kept).
#[actix_web::test]
async fn campaign_simulate_accepts_valid_scenario_on_published_branch_and_marks_version_simulated()
{
    let room = Room::start().await;
    seed_published_usd_rate(&room.pool).await;
    let (campaign_id, version_id) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/simulate");
    let payload = json!({
        "scenarios": [{
            "branchId": "MAIN",
            "currency": "USD",
            "amount": "100.00"
        }]
    });

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("sim_valid"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "valid scenario must answer 200: {}",
        ans.text
    );
    assert!(
        !ans.body["simulationId"].as_str().unwrap_or("").is_empty(),
        "response must include simulationId: {}",
        ans.text
    );
    assert_eq!(
        simulation_count(&room.pool, version_id).await,
        1,
        "valid simulation must insert 1 campaign_simulations row"
    );
    assert_eq!(
        version_status(&room.pool, version_id).await,
        "SIMULATED",
        "valid simulation must move version status from DRAFT to SIMULATED"
    );

    room.finish().await;
}
