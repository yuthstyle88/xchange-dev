//! #2648: `campaignVersionNew` (`POST /api/campaigns/{id}/versions`) answers
//! `409 staleVersion` when the campaign already has an open `DRAFT` or `SIMULATED`
//! version, and does not violate `one_draft_per_campaign` with a 500.

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
        let schema = unique("mk_ver_new_draft");
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
    .expect("seed branch");

    (campaign_id, version_id)
}

// =================================================================================================
// Behaviour Table Row 1: Campaign with an open DRAFT version -> 409 staleVersion;
// still exactly one DRAFT/SIMULATED row
// =================================================================================================
#[actix_web::test]
async fn campaign_with_open_draft_version_answers_409_stale_version() {
    let room = Room::start().await;
    let (campaign_id, _version_id) = seed_campaign(&room.pool, "DRAFT").await;
    let uri = format!("/api/campaigns/{campaign_id}/versions");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("new_ver_draft"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "campaign with open DRAFT version must answer 409 CONFLICT: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "staleVersion",
        "expected staleVersion error code, got: {}",
        ans.text
    );

    let draft_sim_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM campaign_versions \
         WHERE campaign_id = $1 AND status IN ('DRAFT', 'SIMULATED')",
    )
    .bind(campaign_id)
    .fetch_one(&room.pool)
    .await
    .expect("count draft/simulated versions");
    assert_eq!(draft_sim_count, 1, "still exactly one DRAFT/SIMULATED row");

    let total_versions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM campaign_versions WHERE campaign_id = $1")
            .bind(campaign_id)
            .fetch_one(&room.pool)
            .await
            .expect("count total versions");
    assert_eq!(total_versions, 1, "no extra version was created");

    room.finish().await;
}

// =================================================================================================
// Behaviour Table Row 1 (variant): Campaign with APPROVED v1 and open v2 DRAFT
// -> 409 staleVersion; still exactly one DRAFT/SIMULATED row
// =================================================================================================
#[actix_web::test]
async fn campaign_with_approved_v1_and_open_v2_draft_answers_409_stale_version() {
    let room = Room::start().await;
    let (campaign_id, _v1_id) = seed_campaign(&room.pool, "APPROVED").await;
    let org_id = Uuid::parse_str(ORG).unwrap();
    let user_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let v2_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2, $3, 2, 'DRAFT', now() - interval '1 hour', now() + interval '10 days', \
                   10000.00, 'RATE', 1, NULL, $4, now(), now())",
    )
    .bind(v2_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(user_id)
    .execute(&room.pool)
    .await
    .expect("seed v2 draft version");

    let uri = format!("/api/campaigns/{campaign_id}/versions");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("new_ver_approved_with_draft"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "campaign with approved v1 and open v2 draft must answer 409 CONFLICT: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "staleVersion",
        "expected staleVersion error code, got: {}",
        ans.text
    );

    let draft_sim_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM campaign_versions \
         WHERE campaign_id = $1 AND status IN ('DRAFT', 'SIMULATED')",
    )
    .bind(campaign_id)
    .fetch_one(&room.pool)
    .await
    .expect("count draft/simulated versions");
    assert_eq!(draft_sim_count, 1, "still exactly one DRAFT/SIMULATED row");

    let total_versions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM campaign_versions WHERE campaign_id = $1")
            .bind(campaign_id)
            .fetch_one(&room.pool)
            .await
            .expect("count total versions");
    assert_eq!(
        total_versions, 2,
        "no extra version was created beyond v1 and v2"
    );

    room.finish().await;
}

// =================================================================================================
// Behaviour Table Row 2: Campaign whose open version is SIMULATED
// -> 409 staleVersion; nothing written
// =================================================================================================
#[actix_web::test]
async fn campaign_whose_open_version_is_simulated_answers_409_stale_version() {
    let room = Room::start().await;
    let (campaign_id, _version_id) = seed_campaign(&room.pool, "SIMULATED").await;
    let uri = format!("/api/campaigns/{campaign_id}/versions");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("new_ver_sim"))),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::CONFLICT,
        "campaign whose open version is SIMULATED must answer 409 CONFLICT: {}",
        ans.text
    );
    assert_eq!(
        ans.code(),
        "staleVersion",
        "expected staleVersion error code, got: {}",
        ans.text
    );

    let total_versions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM campaign_versions WHERE campaign_id = $1")
            .bind(campaign_id)
            .fetch_one(&room.pool)
            .await
            .expect("count total versions");
    assert_eq!(total_versions, 1, "nothing written to campaign_versions");

    room.finish().await;
}

// =================================================================================================
// Behaviour Table Row 3: Campaign with only non-draft (APPROVED/PAUSED/SUPERSEDED) versions
// -> 200; a new DRAFT version (kept, mk4_campaign_commands_test.rs)
// =================================================================================================
#[actix_web::test]
async fn campaign_with_only_closed_versions_answers_200_new_draft() {
    let room = Room::start().await;
    for status in ["APPROVED", "PAUSED", "SUPERSEDED"] {
        let (campaign_id, _) = seed_campaign(&room.pool, status).await;
        let uri = format!("/api/campaigns/{campaign_id}/versions");

        let ans = call(
            &room,
            Some("MARKETING"),
            Method::POST,
            &uri,
            Some((json!({}), &unique(&format!("new_ver_{status}")))),
        )
        .await;

        assert_eq!(
            ans.status,
            StatusCode::OK,
            "campaign with status {status} must answer 200 OK: {}",
            ans.text
        );
        assert_eq!(ans.body["version"], 2);
        assert_eq!(ans.body["status"], "DRAFT");

        let total_versions: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM campaign_versions WHERE campaign_id = $1")
                .bind(campaign_id)
                .fetch_one(&room.pool)
                .await
                .expect("count total versions");
        assert_eq!(total_versions, 2, "a new draft version was added");
    }

    room.finish().await;
}

// =================================================================================================
// Behaviour Table Row 4: Unknown campaign id -> 404 campaignNotFound, as today (kept)
// =================================================================================================
#[actix_web::test]
async fn unknown_campaign_id_answers_404_campaign_not_found() {
    let room = Room::start().await;
    let unknown_id = Uuid::new_v4();
    let uri = format!("/api/campaigns/{unknown_id}/versions");

    let ans = call(
        &room,
        Some("MARKETING"),
        Method::POST,
        &uri,
        Some((json!({}), &unique("new_ver_unknown"))),
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
        "campaignNotFound",
        "expected campaignNotFound error code, got: {}",
        ans.text
    );

    room.finish().await;
}
