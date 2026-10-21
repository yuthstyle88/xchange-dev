//! #1939 (design `docs/design/blueprint/marketing-campaigns.md` §4.5, §7, :305, :507):
//! `campaignVersionSave` (`PUT /api/campaigns/{id}/draft`) answers `409 staleVersion`
//! when the campaign exists in the caller's organisation but has no editable
//! (`DRAFT` / `SIMULATED`) version, and keeps `404 versionNotFound` for an unknown
//! campaign id or a campaign belonging to another organisation.

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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-000000000099";
const TEST_USER: &str = "00000000-0000-0000-0000-000000000002";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("mk_save_stale");
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
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1, 'ORG2', 'Asia/Bangkok', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(Uuid::parse_str(OTHER_ORG).unwrap())
        .execute(&pool)
        .await
        .expect("seed other organization");

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

async fn seed_campaign_in_org(pool: &PgPool, org_str: &str, status: &str) -> (Uuid, Uuid) {
    let org_id = Uuid::parse_str(org_str).unwrap();
    let user_id = Uuid::parse_str(TEST_USER).unwrap();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let v_hash = if status == "DRAFT" || status == "SIMULATED" {
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
         ) VALUES ($1, $2, $3, 1, $4, '2026-10-01T00:00:00Z'::timestamptz, '2026-10-31T00:00:00Z'::timestamptz, \
                   10000.00, 'RATE', 1, $5, $6, '2026-10-01T00:00:00Z'::timestamptz, '2026-10-01T00:00:00Z'::timestamptz)",
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

fn save_payload() -> Value {
    json!({
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "25000.00",
        "stackGroup": "FEE",
        "priority": 5,
        "branches": ["MAIN"]
    })
}

// =================================================================================================
// §9 Row 1: campaign whose only version is PENDING_APPROVAL -> 409 staleVersion; row is unchanged
// =================================================================================================
#[actix_web::test]
async fn campaign_whose_only_version_is_pending_approval_answers_409_stale_version_and_leaves_row_unchanged()
 {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign_in_org(&room.pool, ORG, "PENDING_APPROVAL").await;
    let uri = format!("/api/campaigns/{campaign_id}/draft");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        &uri,
        Some((save_payload(), &unique("save_pending"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "editing a PENDING_APPROVAL campaign must answer 409 CONFLICT: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "staleVersion",
        "editing a PENDING_APPROVAL campaign must answer staleVersion: {}",
        ans.text
    );

    let row = sqlx::query(
        "SELECT status, budget_thb, stack_group, priority, version_hash, starts_at, ends_at, edited_at \
         FROM campaign_versions WHERE id = $1",
    )
    .bind(version_id)
    .fetch_one(&room.pool)
    .await
    .expect("version row still exists");

    let status: String = row.get("status");
    let budget_thb: Decimal = row.get("budget_thb");
    let stack_group: String = row.get("stack_group");
    let priority: i32 = row.get("priority");
    let version_hash: Option<String> = row.get("version_hash");
    let starts_at: DateTime<Utc> = row.get("starts_at");
    let ends_at: DateTime<Utc> = row.get("ends_at");
    let edited_at: DateTime<Utc> = row.get("edited_at");

    assert_eq!(status, "PENDING_APPROVAL", "status must remain unchanged");
    assert_eq!(
        budget_thb,
        d("10000.00"),
        "budget_thb must remain unchanged"
    );
    assert_eq!(stack_group, "RATE", "stack_group must remain unchanged");
    assert_eq!(priority, 1, "priority must remain unchanged");
    assert_eq!(
        version_hash.as_deref(),
        Some("v1_hash"),
        "version_hash must remain unchanged"
    );
    assert_eq!(
        starts_at.to_rfc3339(),
        "2026-10-01T00:00:00+00:00",
        "starts_at must remain unchanged"
    );
    assert_eq!(
        ends_at.to_rfc3339(),
        "2026-10-31T00:00:00+00:00",
        "ends_at must remain unchanged"
    );
    assert_eq!(
        edited_at.to_rfc3339(),
        "2026-10-01T00:00:00+00:00",
        "edited_at must remain unchanged"
    );

    room.finish().await;
}

// =================================================================================================
// §9 Row 2: campaign whose only version is APPROVED -> 409 staleVersion
// =================================================================================================
#[actix_web::test]
async fn campaign_whose_only_version_is_approved_answers_409_stale_version() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign_in_org(&room.pool, ORG, "APPROVED").await;
    let uri = format!("/api/campaigns/{campaign_id}/draft");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        &uri,
        Some((save_payload(), &unique("save_approved"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "editing an APPROVED campaign must answer 409 CONFLICT: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "staleVersion",
        "editing an APPROVED campaign must answer staleVersion: {}",
        ans.text
    );

    let (status, budget_thb): (String, Decimal) =
        sqlx::query_as("SELECT status, budget_thb FROM campaign_versions WHERE id = $1")
            .bind(version_id)
            .fetch_one(&room.pool)
            .await
            .expect("version row still exists");
    assert_eq!(status, "APPROVED");
    assert_eq!(budget_thb, d("10000.00"));

    room.finish().await;
}

// =================================================================================================
// §9 Row 3: campaign with a DRAFT -> saves as today (200)
// =================================================================================================
#[actix_web::test]
async fn campaign_with_a_draft_saves_as_today_200() {
    let room = Room::start().await;
    let (campaign_id, version_id) = seed_campaign_in_org(&room.pool, ORG, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/draft");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        &uri,
        Some((save_payload(), &unique("save_draft"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "editing a DRAFT campaign must answer 200 OK: {}",
        ans.text
    );
    assert_eq!(ans.body["versionId"], version_id.to_string());
    assert_eq!(ans.body["version"], 1);
    assert_eq!(ans.body["status"], "DRAFT");

    let (status, budget_thb, stack_group, priority): (String, Decimal, String, i32) =
        sqlx::query_as(
            "SELECT status, budget_thb, stack_group, priority FROM campaign_versions WHERE id = $1",
        )
        .bind(version_id)
        .fetch_one(&room.pool)
        .await
        .expect("updated version row");
    assert_eq!(status, "DRAFT");
    assert_eq!(budget_thb, d("25000.00"));
    assert_eq!(stack_group, "FEE");
    assert_eq!(priority, 5);

    room.finish().await;
}

// =================================================================================================
// §9 Row 4: unknown campaign id -> 404 versionNotFound
// =================================================================================================
#[actix_web::test]
async fn unknown_campaign_id_answers_404_version_not_found() {
    let room = Room::start().await;
    let unknown_id = Uuid::new_v4();
    let uri = format!("/api/campaigns/{unknown_id}/draft");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::PUT,
        &uri,
        Some((save_payload(), &unique("save_unknown"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "unknown campaign id must answer 404 NOT_FOUND: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "versionNotFound",
        "unknown campaign id must answer versionNotFound: {}",
        ans.text
    );

    room.finish().await;
}

// =================================================================================================
// §9 Row 5: campaign of another organisation -> 404 versionNotFound
// =================================================================================================
#[actix_web::test]
async fn campaign_of_another_organisation_answers_404_version_not_found() {
    let room = Room::start().await;
    // Seed both a PENDING_APPROVAL and a DRAFT campaign in OTHER_ORG so that
    // omitting `organization_id` from either query would leak or mutate across orgs.
    let (other_pending_id, other_pending_ver) =
        seed_campaign_in_org(&room.pool, OTHER_ORG, "PENDING_APPROVAL").await;
    let (other_draft_id, other_draft_ver) =
        seed_campaign_in_org(&room.pool, OTHER_ORG, "DRAFT").await;

    for (label, cid) in [
        ("other-org PENDING_APPROVAL", other_pending_id),
        ("other-org DRAFT", other_draft_id),
    ] {
        let uri = format!("/api/campaigns/{cid}/draft");
        let ans = call(
            &room,
            Some("MARKETING"),
            Method::PUT,
            &uri,
            Some((save_payload(), &unique("save_other_org"))),
        )
        .await;

        assert_eq!(
            ans.status,
            StatusCode::NOT_FOUND,
            "{label} campaign must answer 404 NOT_FOUND (never leak staleVersion or 200): {}",
            ans.text
        );
        assert_eq!(
            ans.code(),
            "versionNotFound",
            "{label} campaign must answer versionNotFound: {}",
            ans.text
        );
    }

    let pending_budget: Decimal =
        sqlx::query_scalar("SELECT budget_thb FROM campaign_versions WHERE id = $1")
            .bind(other_pending_ver)
            .fetch_one(&room.pool)
            .await
            .unwrap();
    let draft_budget: Decimal =
        sqlx::query_scalar("SELECT budget_thb FROM campaign_versions WHERE id = $1")
            .bind(other_draft_ver)
            .fetch_one(&room.pool)
            .await
            .unwrap();
    assert_eq!(pending_budget, d("10000.00"));
    assert_eq!(draft_budget, d("10000.00"));

    room.finish().await;
}
