//! MK-4 (#1397, design `docs/design/blueprint/marketing-campaigns.md` §4.1, §4.4, §4.5, §6, §7, D-4, D-5):
//! Up-front tests for MK-4 campaign command routes through the real v3 seam.
//!
//! Written before the implementation by the test writer (gemini/pool-4, on the BE lane controller
//! claude/ad05bf8d's order); the implementer must not edit, delete or weaken them.
//!
//! Routes and behaviours tested:
//! 1. `campaignCreate` (`POST /api/campaigns`): refuses role without `MarketingCampaigns` (TELLER -> 400).
//! 2. `campaignVersionSave` (`PUT /api/campaigns/{id}/draft`): refuses TELLER -> 400.
//! 3. `campaignVersionNew` (`POST /api/campaigns/{id}/versions`): refuses TELLER -> 400.
//! 4. `campaignSimulate` (`POST /api/campaigns/{id}/simulate`): refuses TELLER -> 400.
//! 5. `campaignSubmit` (`POST /api/campaigns/{id}/submit`): refuses TELLER -> 400.
//! 6. `campaignPause` (`POST /api/campaigns/{id}/pause`): refuses TELLER -> 400.
//! 7. `campaignResume` (`POST /api/campaigns/{id}/resume`): refuses TELLER -> 400.
//! 8. `couponIssue` (`POST /api/campaigns/{id}/coupons`): refuses TELLER -> 400.
//! 9. `marginGuardSet` (`PUT /api/margin-guard`): refuses MARKETING -> 400 (needs `RatePolicySave`); EXECUTIVE passes.
//! 10. Every command enforces `Idempotency-Key` conflict: same key with different payload -> 409 `idempotencyConflict`.
//! 11. `couponIssue` replay: same key returns same coupon ids, but codes are shown only on the first response (§7).
//! 12. Approve n+1 while quotes hold reservations on n -> existing reservations keep n's id (§7, §9 edge case 4).
//! 13. Simulator applies the rule to the published line (D-7, §4.5, §6).

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

use xc_approval::domain::request::Outcome;
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_marketing::application::approval::CampaignVersionSubject;
use xc_marketing::application::campaign_service::CampaignService;
use xc_marketing::application::simulator::{CampaignSimulator, SimulationScenario};
use xc_marketing::domain::campaign::version::VersionTerms;
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

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("valid decimal")
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
        let schema = unique("mk4_routes");
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
    if let Some(cust) = spec.strip_prefix("CUSTOMER:") {
        return Some(Actor::new(
            "user-cust",
            ORG,
            vec![ScopedRole::new(Role::Customer, None, from, None)],
            Some(cust),
        ));
    }
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
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                user,
                staff_user,
            }
        })
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

    (campaign_id, version_id)
}

// ------------------------------------------------------------
// Criterion 3: RoleNotAllowed = 400 for unauthorized roles & command execution
// ------------------------------------------------------------

#[actix_web::test]
async fn campaign_create_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    let payload = json!({
        "name": "Summer Promo",
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "10000.00",
        "stackGroup": "RATE",
        "priority": 1,
        "branches": ["MAIN"]
    });

    let ans = call(
        &room,
        Some("TELLER"),
        Method::POST,
        "/api/campaigns",
        Some((payload.clone(), &unique("create_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignCreate must refuse TELLER with 400 (criterion 3): {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes command and persists campaign
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        "/api/campaigns",
        Some((payload, &unique("create_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignCreate must accept MARKETING"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM campaigns WHERE name = 'Summer Promo'")
            .fetch_one(&room.pool)
            .await
            .unwrap();
    assert_eq!(
        count, 1,
        "campaignCreate must persist campaign record into database"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_version_save_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    let (campaign_id, _) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/draft");
    let payload = json!({
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "20000.00",
        "stackGroup": "RATE",
        "priority": 2,
        "branches": ["MAIN"]
    });

    let ans = call(
        &room,
        Some("TELLER"),
        Method::PUT,
        &uri,
        Some((payload.clone(), &unique("save_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignVersionSave must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes command and returns updated version 1
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        &uri,
        Some((payload, &unique("save_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignVersionSave must accept MARKETING"
    );
    assert_eq!(
        auth_ans.body["version"], 1,
        "campaignVersionSave must return version 1"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_version_new_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    let (campaign_id, _) = seed_campaign(&room.pool, "APPROVED").await;
    let uri = format!("/api/campaigns/{campaign_id}/versions");
    let ans = call(
        &room,
        Some("TELLER"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("new_ver_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignVersionNew must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes command and returns version 2
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("new_ver_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignVersionNew must accept MARKETING"
    );
    assert_eq!(
        auth_ans.body["version"], 2,
        "campaignVersionNew must create and return version 2"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_simulate_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    let (campaign_id, _) = seed_campaign(&room.pool, "DRAFT").await;
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
        Some("TELLER"),
        Method::POST,
        &uri,
        Some((payload.clone(), &unique("sim_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignSimulate must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes simulate and records simulation row (§7 [required])
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("sim_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignSimulate must accept MARKETING"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM campaign_simulations")
        .fetch_one(&room.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "campaignSimulate must record simulation row into campaign_simulations table (§7 [required])"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_submit_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    let (campaign_id, v1_id) = seed_campaign(&room.pool, "DRAFT").await;

    // Simulate before submit (against D-4 simulationRequired)
    sqlx::query(
        "INSERT INTO campaign_simulations (id, campaign_version_id, input, result, cost_source, simulated_by, simulated_at) \
         VALUES ($1, $2, '{}'::jsonb, '{}'::jsonb, 'UNAVAILABLE', '00000000-0000-0000-0000-000000000002', now())",
    )
    .bind(Uuid::new_v4())
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .unwrap();

    let uri = format!("/api/campaigns/{campaign_id}/submit");
    let ans = call(
        &room,
        Some("TELLER"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("submit_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignSubmit must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes submit and transitions to PENDING_APPROVAL
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("submit_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignSubmit must accept MARKETING"
    );
    assert_eq!(
        auth_ans.body["status"], "PENDING_APPROVAL",
        "campaignSubmit must transition version to PENDING_APPROVAL"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_pause_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    // Approves before pause
    let (campaign_id, _) = seed_campaign(&room.pool, "APPROVED").await;
    let uri = format!("/api/campaigns/{campaign_id}/pause");
    let ans = call(
        &room,
        Some("TELLER"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("pause_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignPause must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes pause and transitions to PAUSED
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("pause_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignPause must accept MARKETING"
    );
    assert_eq!(
        auth_ans.body["status"], "PAUSED",
        "campaignPause must transition version to PAUSED"
    );

    room.finish().await;
}

#[actix_web::test]
async fn campaign_resume_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    // Pauses before resume
    let (campaign_id, _) = seed_campaign(&room.pool, "PAUSED").await;
    let uri = format!("/api/campaigns/{campaign_id}/resume");
    let ans = call(
        &room,
        Some("TELLER"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("resume_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "campaignResume must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes resume and transitions to APPROVED
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("resume_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "campaignResume must accept MARKETING"
    );
    assert_eq!(
        auth_ans.body["status"], "APPROVED",
        "campaignResume must transition version to APPROVED"
    );

    room.finish().await;
}

#[actix_web::test]
async fn coupon_issue_refuses_teller_with_400_role_not_allowed() {
    let room = Room::start().await;
    // Seed APPROVED campaign with a real coupon-requiring rule
    let (campaign_id, v1_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{}', '{}', '{}', true, 1)",
    )
    .bind(rule_id)
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .expect("seed coupon rule");

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 5,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans = call(
        &room,
        Some("TELLER"),
        Method::POST,
        &uri,
        Some((payload.clone(), &unique("coupon_teller"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "couponIssue must refuse TELLER with 400: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        ans.text
    );

    // Authorized MARKETING: executes couponIssue and returns issued coupons
    let auth_ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &unique("coupon_mktg"))),
    )
    .await;
    assert_eq!(
        auth_ans.status,
        StatusCode::OK,
        "couponIssue must accept MARKETING"
    );
    assert_eq!(
        auth_ans.body["coupons"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        5,
        "couponIssue must issue 5 coupons"
    );

    room.finish().await;
}

#[actix_web::test]
async fn margin_guard_set_refuses_marketing_with_400_and_accepts_executive() {
    let room = Room::start().await;

    // MARKETING lacks RatePolicySave -> 400 roleNotAllowedHere
    let mktg_ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "0.5000" }), &unique("guard_mktg"))),
    )
    .await;

    assert_eq!(
        mktg_ans.status,
        StatusCode::BAD_REQUEST,
        "marginGuardSet must refuse MARKETING with 400 (needs RatePolicySave): {}",
        mktg_ans.text
    );
    assert_eq!(
        mktg_ans.code(),
        "roleNotAllowedHere",
        "expected roleNotAllowedHere code, got: {}",
        mktg_ans.text
    );

    // EXECUTIVE has RatePolicySave -> 200 and persists record
    let exec_ans = call(
        &room,
        Some("EXECUTIVE"),
        Method::PUT,
        "/api/margin-guard",
        Some((json!({ "maxSpreadShare": "0.5000" }), &unique("guard_exec"))),
    )
    .await;

    assert_eq!(
        exec_ans.status,
        StatusCode::OK,
        "marginGuardSet must accept EXECUTIVE: {}",
        exec_ans.text
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM margin_guards")
        .fetch_one(&room.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "marginGuardSet must persist margin guard record into database"
    );

    room.finish().await;
}

// ------------------------------------------------------------
// Criterion 4: Idempotency-Key conflict and replay across all 9 commands
// ------------------------------------------------------------

async fn assert_idempotency_conflict(
    room: &Room,
    who: &str,
    method: Method,
    uri: &str,
    payload_a: Value,
    payload_b: Value,
    cmd_name: &str,
) {
    let idem_key = unique(&format!("idem_{cmd_name}"));
    let ans1 = call(
        room,
        Some(who),
        method.clone(),
        uri,
        Some((payload_a, &idem_key)),
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "{cmd_name}: first call must succeed"
    );

    let ans2 = call(room, Some(who), method, uri, Some((payload_b, &idem_key))).await;
    assert_eq!(
        ans2.status,
        StatusCode::CONFLICT,
        "{cmd_name}: second call with same idempotency key and different body must yield 409 idempotencyConflict (criterion 4): {}",
        ans2.text
    );
    assert_eq!(
        ans2.code(),
        "idempotencyConflict",
        "{cmd_name}: error code must be idempotencyConflict"
    );
}

#[actix_web::test]
async fn commands_enforce_idempotency_conflict_on_different_payload() {
    let room = Room::start().await;

    // 1. campaignCreate
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        "/api/campaigns",
        json!({
            "name": "Promo Alpha",
            "startsAt": "2026-11-01T00:00:00Z",
            "endsAt": "2026-12-01T00:00:00Z",
            "budgetThb": "10000.00",
            "stackGroup": "DEFAULT",
            "priority": 1,
            "branches": ["MAIN"]
        }),
        json!({
            "name": "Promo Beta (Different Payload)",
            "startsAt": "2026-11-01T00:00:00Z",
            "endsAt": "2026-12-01T00:00:00Z",
            "budgetThb": "20000.00",
            "stackGroup": "DEFAULT",
            "priority": 2,
            "branches": ["MAIN"]
        }),
        "campaignCreate",
    )
    .await;

    // 2. campaignVersionSave
    let (c2, _) = seed_campaign(&room.pool, "DRAFT").await;
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::PUT,
        &format!("/api/campaigns/{c2}/draft"),
        json!({
            "startsAt": "2026-11-01T00:00:00Z",
            "endsAt": "2026-12-01T00:00:00Z",
            "budgetThb": "10000.00",
            "stackGroup": "RATE",
            "priority": 1,
            "branches": ["MAIN"]
        }),
        json!({
            "startsAt": "2026-11-01T00:00:00Z",
            "endsAt": "2026-12-01T00:00:00Z",
            "budgetThb": "20000.00",
            "stackGroup": "RATE",
            "priority": 2,
            "branches": ["MAIN"]
        }),
        "campaignVersionSave",
    )
    .await;

    // 3. campaignVersionNew
    let (c3, _) = seed_campaign(&room.pool, "APPROVED").await;
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        &format!("/api/campaigns/{c3}/versions"),
        json!({ "budgetThb": "10000.00" }),
        json!({ "budgetThb": "20000.00" }),
        "campaignVersionNew",
    )
    .await;

    // 4. campaignSimulate
    let (c4, _) = seed_campaign(&room.pool, "DRAFT").await;
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        &format!("/api/campaigns/{c4}/simulate"),
        json!({
            "scenarios": [{
                "branchId": "MAIN",
                "currency": "USD",
                "amount": "100.00"
            }]
        }),
        json!({
            "scenarios": [{
                "branchId": "MAIN",
                "currency": "EUR",
                "amount": "200.00"
            }]
        }),
        "campaignSimulate",
    )
    .await;

    // 5. campaignSubmit
    let (c5, v5) = seed_campaign(&room.pool, "DRAFT").await;
    sqlx::query(
        "INSERT INTO campaign_simulations (id, campaign_version_id, input, result, cost_source, simulated_by, simulated_at) \
         VALUES ($1, $2, '{}'::jsonb, '{}'::jsonb, 'UNAVAILABLE', '00000000-0000-0000-0000-000000000002', now())",
    )
    .bind(Uuid::new_v4())
    .bind(v5)
    .execute(&room.pool)
    .await
    .unwrap();
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        &format!("/api/campaigns/{c5}/submit"),
        json!({ "name": "Submission Alpha" }),
        json!({ "name": "Submission Beta" }),
        "campaignSubmit",
    )
    .await;

    // 6. campaignPause
    let (c6, _) = seed_campaign(&room.pool, "APPROVED").await;
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        &format!("/api/campaigns/{c6}/pause"),
        json!({ "name": "Pause Alpha" }),
        json!({ "name": "Pause Beta" }),
        "campaignPause",
    )
    .await;

    // 7. campaignResume
    let (c7, _) = seed_campaign(&room.pool, "PAUSED").await;
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        &format!("/api/campaigns/{c7}/resume"),
        json!({ "name": "Resume Alpha" }),
        json!({ "name": "Resume Beta" }),
        "campaignResume",
    )
    .await;

    // 8. couponIssue
    let (c8, v8) = seed_campaign(&room.pool, "APPROVED").await;
    let r8 = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{}', '{}', '{}', true, 1)",
    )
    .bind(r8)
    .bind(v8)
    .execute(&room.pool)
    .await
    .unwrap();
    assert_idempotency_conflict(
        &room,
        "MARKETING",
        Method::POST,
        &format!("/api/campaigns/{c8}/coupons"),
        json!({
            "ruleId": r8.to_string(),
            "count": 2,
            "expiresAt": "2026-12-31T23:59:59Z"
        }),
        json!({
            "ruleId": r8.to_string(),
            "count": 5,
            "expiresAt": "2026-12-31T23:59:59Z"
        }),
        "couponIssue",
    )
    .await;

    // 9. marginGuardSet
    assert_idempotency_conflict(
        &room,
        "EXECUTIVE",
        Method::PUT,
        "/api/margin-guard",
        json!({ "maxSpreadShare": "0.5000" }),
        json!({ "maxSpreadShare": "0.4000" }),
        "marginGuardSet",
    )
    .await;

    room.finish().await;
}

// ------------------------------------------------------------
// §9 edge case 6: couponIssue replay returns same IDs, codes only first time
// ------------------------------------------------------------

#[actix_web::test]
async fn coupon_issue_replay_returns_same_ids_and_omits_codes_on_second_call() {
    let room = Room::start().await;
    let (campaign_id, v1_id) = seed_campaign(&room.pool, "APPROVED").await;
    let rule_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{}', '{}', '{}', true, 1)",
    )
    .bind(rule_id)
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .expect("seed rule");

    let uri = format!("/api/campaigns/{campaign_id}/coupons");
    let idem_key = unique("coupon_replay");

    let payload = json!({
        "ruleId": rule_id.to_string(),
        "count": 2,
        "expiresAt": "2026-12-31T23:59:59Z"
    });

    let ans1 = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload.clone(), &idem_key)),
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "first couponIssue must succeed"
    );

    let coupons_1 = ans1.body["coupons"]
        .as_array()
        .expect("coupons array in response");
    assert_eq!(coupons_1.len(), 2, "must return 2 issued coupons");
    assert!(
        coupons_1[0]["code"].as_str().is_some()
            && !coupons_1[0]["code"].as_str().unwrap().is_empty(),
        "first call must return generated coupon code plaintext"
    );

    // Replay with identical key and payload
    let ans2 = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((payload, &idem_key)),
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "replay must return cached answer 200"
    );

    let coupons_2 = ans2.body["coupons"]
        .as_array()
        .expect("coupons array in replay");
    assert_eq!(
        coupons_2.len(),
        2,
        "replay must return the same number of coupons"
    );
    assert_eq!(
        coupons_1[0]["id"], coupons_2[0]["id"],
        "replay must return the same coupon ids"
    );
    assert_eq!(
        coupons_1[1]["id"], coupons_2[1]["id"],
        "replay must return the same coupon ids"
    );

    // Codes must be omitted / null on replay (§7, §8, §9 edge case 6)
    assert!(
        coupons_2[0]["code"].is_null() || coupons_2[0]["code"].as_str().unwrap_or("").is_empty(),
        "codes must be shown only the first time and omitted on replay (§7)"
    );

    room.finish().await;
}

// ------------------------------------------------------------
// §9 edge case 4: approving n+1 keeps reservations on n intact
// ------------------------------------------------------------

#[actix_web::test]
async fn approving_new_version_keeps_reservations_on_previous_version_intact() {
    let room = Room::start().await;
    let org_id = Uuid::parse_str(ORG).unwrap();
    let campaign_id = Uuid::new_v4();
    let v1_id = Uuid::new_v4();
    let v2_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();
    let quote_id = Uuid::new_v4();
    let res_id = Uuid::new_v4();
    let _now = Utc::now();

    // Seed campaign with version 1 APPROVED
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2, 'Reservation Retention', NULL, '00000000-0000-0000-0000-000000000002', now())",
    )
    .bind(campaign_id)
    .bind(org_id)
    .execute(&room.pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 1, 'APPROVED', now() - interval '1 hour', now() + interval '10 days', \
                   10000.00, 'DEFAULT', 1, 'hash_v1', '00000000-0000-0000-0000-000000000002', now(), now())",
    )
    .bind(v1_id)
    .bind(org_id)
    .bind(campaign_id)
    .execute(&room.pool)
    .await
    .expect("seed version 1");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(v1_id)
        .bind(campaign_id)
        .execute(&room.pool)
        .await
        .expect("set current_version_id to v1");

    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{}', '{}', '{}', 1)",
    )
    .bind(rule_id)
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .expect("seed rule");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
         VALUES ($1, 0, 0)",
    )
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .expect("seed budget v1");

    // Seed quote and active reservation on version 1
    let pub_request = Uuid::new_v4();
    let publication = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
         policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ($1, $2, 'RATE_PUBLICATION', 'MAIN', $3, 'hash_pub', '1', '{}'::jsonb, 'test', now(), 'APPROVED', $4)",
    )
    .bind(pub_request)
    .bind(org_id)
    .bind(publication.to_string())
    .bind(unique("pub"))
    .execute(&room.pool)
    .await
    .expect("seed approval_request for publication");

    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
         approved_request_id, published_by) \
         VALUES ($1, $2, 'MAIN', 1, now(), $3, 'test')",
    )
    .bind(publication)
    .bind(org_id)
    .bind(pub_request)
    .execute(&room.pool)
    .await
    .expect("seed publication");

    sqlx::query(
        "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, publication_id, \
         expires_at, created_by, created_at, updated_at) \
         VALUES ($1, $2, 'MAIN', 'INDICATIVE', 'DESK', 'ACTIVE', $3, now() + interval '15 minutes', \
         'test', now(), now())",
    )
    .bind(quote_id)
    .bind(org_id)
    .bind(publication)
    .execute(&room.pool)
    .await
    .expect("seed quote");

    sqlx::query(
        "INSERT INTO benefit_reservations ( \
             id, organization_id, rule_id, campaign_version_id, quote_id, amount_thb, \
             status, expires_at, created_at \
         ) VALUES ($1, $2, $3, $4, $5, 50.00, 'RESERVED', now() + interval '10 minutes', now())",
    )
    .bind(res_id)
    .bind(org_id)
    .bind(rule_id)
    .bind(v1_id)
    .bind(quote_id)
    .execute(&room.pool)
    .await
    .expect("seed reservation on v1");

    // Version 2 is submitted and approved through approval subject hook
    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 2, 'PENDING_APPROVAL', now(), now() + interval '10 days', \
                   15000.00, 'DEFAULT', 1, 'hash_v2', '00000000-0000-0000-0000-000000000002', now(), now())",
    )
    .bind(v2_id)
    .bind(org_id)
    .bind(campaign_id)
    .execute(&room.pool)
    .await
    .expect("seed version 2");

    let subject = CampaignVersionSubject::new(room.pool.clone());
    let mut tx = room.pool.begin().await.expect("begin tx");
    subject
        .on_decided(&mut tx, &v2_id.to_string(), Outcome::Approved)
        .await
        .expect("on_decided succeeds");
    tx.commit().await.expect("commit tx");

    let current_v: Option<Uuid> =
        sqlx::query_scalar("SELECT current_version_id FROM campaigns WHERE id = $1")
            .bind(campaign_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch current_version_id");
    assert_eq!(
        current_v,
        Some(v2_id),
        "approving version 2 must set campaigns.current_version_id to v2_id"
    );

    // Assert: the reservation still retains version 1's id (§7, §9 edge case 4)
    let reserved_version: Uuid =
        sqlx::query_scalar("SELECT campaign_version_id FROM benefit_reservations WHERE id = $1")
            .bind(res_id)
            .fetch_one(&room.pool)
            .await
            .expect("fetch reservation campaign_version_id");

    assert_eq!(
        reserved_version, v1_id,
        "reservations held on version n must keep n's id when version n+1 is approved (§7, §9 edge case 4)"
    );

    room.finish().await;
}

fn sample_terms(now: chrono::DateTime<Utc>) -> VersionTerms {
    VersionTerms {
        starts_at: now + chrono::Duration::hours(1),
        ends_at: now + chrono::Duration::days(30),
        budget_thb: d("50000.00"),
        max_uses: Some(500),
        stack_group: "DEFAULT".to_string(),
        priority: 10,
        segment_snapshot_id: None,
    }
}

// ------------------------------------------------------------
// 13. Simulator applies the rule to the published line (D-7, §4.5, §6)
// ------------------------------------------------------------

#[actix_web::test]
async fn simulator_applies_the_rule_to_the_published_line() {
    let room = Room::start().await;
    let service = CampaignService::new(room.pool.clone());
    let simulator = CampaignSimulator::new(room.pool.clone());
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let now = Utc::now();

    let campaign = service
        .create(
            org_id,
            "Simulator Match Campaign",
            sample_terms(now),
            &[MAIN],
            user_id,
            now,
        )
        .await
        .expect("create succeeds");

    let v1_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM campaign_versions WHERE campaign_id = $1 AND version = 1",
    )
    .bind(campaign.id)
    .fetch_one(&room.pool)
    .await
    .expect("version 1 exists");

    // Add a rate improvement rule of 0.25 delta on USD
    let rule_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules ( \
             id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, rule_no \
         ) VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{\"currencies\":[\"USD\"]}', '{\"delta\":\"0.25\"}', '{}', 1)",
    )
    .bind(rule_id)
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .expect("insert benefit rule");

    // Make version APPROVED and active on campaign (Must-fix 5)
    sqlx::query(
        "UPDATE campaign_versions SET status = 'APPROVED', version_hash = 'sim_hash' WHERE id = $1",
    )
    .bind(v1_id)
    .execute(&room.pool)
    .await
    .expect("set version APPROVED");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(v1_id)
        .bind(campaign.id)
        .execute(&room.pool)
        .await
        .expect("set current_version_id");

    // Seed a real rate publication for MAIN branch and USD
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
    .execute(&room.pool)
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
    .execute(&room.pool)
    .await
    .expect("seed rate_publications");

    let snapshot = serde_json::json!({
        "currency": "USD",
        "buy_mode": "MANUAL",
        "sell_mode": "MANUAL",
        "reason": "set for simulation parity test",
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
    .execute(&room.pool)
    .await
    .expect("seed published_rate_lines");

    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = 'MAIN'")
        .bind(publication)
        .execute(&room.pool)
        .await
        .expect("update branch current_publication_id");

    // Call simulator for the scenario
    let scenario = SimulationScenario {
        branch_id: MAIN.to_string(),
        currency: "USD".to_string(),
        amount: d("100.00"),
    };

    let sim_result = simulator
        .simulate(org_id, v1_id, vec![scenario.clone()], user_id, now)
        .await
        .expect("simulation succeeds");

    assert!(
        sim_result.evaluation.is_some(),
        "simulation evaluation must be present"
    );
    let sim_eval = sim_result.evaluation.as_ref().unwrap();
    let applied = &sim_eval.snapshot.applied;
    assert!(
        !applied.is_empty(),
        "simulator must evaluate and apply the eligible rate improvement rule"
    );

    let base_rate = "33.5000";
    let expected_rate_after = "33.7500";
    let expected_diff_thb = "25.00";

    assert_eq!(
        applied[0].rate_after, expected_rate_after,
        "simulator line rate_after must equal published base rate + 0.25 as string"
    );
    assert_ne!(
        applied[0].rate_after, base_rate,
        "benefit rate_after must differ from base rate when delta rule is applied"
    );
    assert_eq!(
        applied[0].amount_thb, expected_diff_thb,
        "simulator benefit amount must equal the THB difference for the scenario quantity as string"
    );

    room.finish().await;
}
