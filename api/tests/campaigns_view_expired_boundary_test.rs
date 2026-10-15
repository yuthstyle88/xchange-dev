//! Ticket #1940 — `campaigns` view status derivation at the `ends_at` boundary
//! (`docs/design/blueprint/marketing-campaigns.md:72, :246`:
//! `ACTIVE (starts_at <= now < ends_at)` · `EXPIRED (now >= ends_at)`).
//!
//! §9 Edge cases (one test each):
//! 1. APPROVED, `now == ends_at` -> `EXPIRED`
//! 2. APPROVED, `now == starts_at` -> `ACTIVE`
//! 3. APPROVED, `now < starts_at` -> `SCHEDULED`
//! 4. APPROVED, `now` just before `ends_at` -> `ACTIVE`
//! 5. PAUSED (any `now`) -> `PAUSED`

mod common;

pub use xchange::bootstrap;
pub mod presentation {
    pub use xchange::presentation::dto;
    pub mod http {
        pub mod v3 {
            pub use xchange::presentation::http::v3::error;
            pub mod view {
                use std::future::Future;

                use actix_web::{HttpRequest, HttpResponse};
                use pos_error::AppResult;
                use xc_auth::domain::access::Actor;
                use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
                use xc_kernel::ids::BranchId;
                use xchange::presentation::http::v3::error::V3Error;
                pub use xchange::presentation::http::v3::view::ViewAnswer;

                pub struct CallerStub {
                    pub user: Option<Actor>,
                }

                pub trait ViewFilter:
                    serde::de::DeserializeOwned
                    + ts_rs::TS
                    + Default
                    + Send
                    + Sync
                    + 'static
                {
                    fn check(&self) -> Result<(), V3Error> {
                        Ok(())
                    }
                }

                pub trait BranchView: Send + Sync + 'static {
                    const NAME: &'static str;
                    const INHERITS: (EntryPath, &'static str);
                    const VERSIONED: bool = true;
                    type Data: serde::Serialize + ts_rs::TS + Send;
                    type Filter: ViewFilter;
                    fn load(
                        &self,
                        branch: &BranchId,
                        who: &Actor,
                        filter: &Self::Filter,
                    ) -> impl Future<Output = AppResult<Self::Data>> + Send;
                    fn load_as(
                        &self,
                        branch: &BranchId,
                        caller: &CallerStub,
                        filter: &Self::Filter,
                    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
                        async move {
                            let who = caller.user.as_ref().expect("caller user");
                            self.load(branch, who, filter).await
                        }
                    }
                }

                pub(crate) fn parse_branch(_raw: Option<&str>) -> Result<Option<BranchId>, ()> {
                    Ok(None)
                }

                pub async fn require_branch(
                    _req: &HttpRequest,
                    _branch: &BranchId,
                ) -> Result<(), V3Error> {
                    Ok(())
                }

                pub async fn authenticate_caller(
                    _req: &HttpRequest,
                    _inherits: (EntryPath, &'static str),
                ) -> Result<CallerStub, HttpResponse> {
                    Err(HttpResponse::Unauthorized().finish())
                }
            }
        }
    }
}

#[rustfmt::skip]
#[allow(dead_code)]
#[path = "../src/presentation/http/v3/views/campaigns.rs"]
mod campaigns_view;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::AppState;
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

use campaigns_view::derive_version_status;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const USER: &str = "00000000-0000-0000-0000-000000000002";
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

fn t(hour: u32, min: u32, sec: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 15, hour, min, sec).unwrap()
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
        let role = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .filter(|&s| s == "MARKETING")
            .map(|_| Role::Marketing);
        let user = role.map(|r| {
            Actor::new(
                "user-marketing",
                ORG,
                vec![ScopedRole::new(r, None, DateTime::<Utc>::UNIX_EPOCH, None)],
                None,
            )
        });
        let staff_user = user.clone();
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

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
    branch: String,
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
        let schema = unique("c_exp");
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
             VALUES ($1::uuid, $2::uuid, 'Test User', 'ACTIVE', 1, now(), 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(USER)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("seed test user");

        let branch = unique("B").replace('_', "-");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch");

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
            branch,
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

    async fn seed_campaign(&self, status: &str, starts_offset: &str, ends_offset: &str) -> Uuid {
        let campaign = Uuid::new_v4();
        let version = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, $4::uuid, now())",
        )
        .bind(campaign)
        .bind(ORG)
        .bind(unique("campaign"))
        .bind(USER)
        .execute(&self.pool)
        .await
        .expect("insert campaign");

        sqlx::query(AssertSqlSafe(format!(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, max_uses, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, 1, $4, now() + interval '{starts_offset}', \
             now() + interval '{ends_offset}', 1000.00, 10, 'g', 1, 'hash', now(), $5::uuid, now())"
        )))
        .bind(version)
        .bind(ORG)
        .bind(campaign)
        .bind(status)
        .bind(USER)
        .execute(&self.pool)
        .await
        .expect("insert campaign_version");

        sqlx::query("INSERT INTO campaign_budgets (campaign_version_id) VALUES ($1)")
            .bind(version)
            .execute(&self.pool)
            .await
            .expect("insert campaign_budget");

        sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
            .bind(version)
            .bind(campaign)
            .execute(&self.pool)
            .await
            .expect("set current_version_id");

        campaign
    }

    async fn fetch_campaign_status(&self, campaign_id: Uuid) -> String {
        let today = Utc::now().date_naive();
        let from = today - chrono::Duration::days(3);
        let to = today + chrono::Duration::days(1);
        let uri = format!(
            "/api/v3/views/campaigns?branch={}&from={from}&to={to}",
            self.branch
        );
        let state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
        let guard = self.guard.clone();
        let app = atest::init_service(
            App::new()
                .app_data(web::Data::from(guard))
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::default()
            .method(Method::GET)
            .uri(&uri)
            .insert_header((header::HOST, HOST))
            .insert_header((header::ORIGIN, SAME_ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((AS, "MARKETING"))
            .to_request();
        let resp = atest::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = atest::read_body(resp).await;
        let body: Value = serde_json::from_slice(&bytes).expect("valid json");
        let rows = body["data"]["campaigns"]
            .as_array()
            .expect("data.campaigns array");
        let row = rows
            .iter()
            .find(|r| r["id"].as_str() == Some(&campaign_id.to_string()))
            .unwrap_or_else(|| panic!("missing campaign {campaign_id} in {rows:?}"));
        row["status"].as_str().expect("status string").to_owned()
    }
}

/// §9 Row 1: APPROVED, `now == ends_at` -> `EXPIRED` (red on `origin/main` where `now > ends_at` returned `ACTIVE`).
#[actix_web::test]
async fn approved_at_exact_ends_at_is_expired() {
    let starts_at = t(9, 0, 0);
    let ends_at = t(17, 0, 0);
    assert_eq!(
        derive_version_status("APPROVED", starts_at, ends_at, ends_at),
        "EXPIRED",
        "marketing-campaigns.md:72, :246: EXPIRED when now >= ends_at"
    );

    let room = Room::start().await;
    let id = room
        .seed_campaign("APPROVED", "-1 hour", "-1 millisecond")
        .await;
    assert_eq!(room.fetch_campaign_status(id).await, "EXPIRED");
    room.finish().await;
}

/// §9 Row 2: APPROVED, `now == starts_at` -> `ACTIVE`.
#[actix_web::test]
async fn approved_at_exact_starts_at_is_active() {
    let starts_at = t(9, 0, 0);
    let ends_at = t(17, 0, 0);
    assert_eq!(
        derive_version_status("APPROVED", starts_at, ends_at, starts_at),
        "ACTIVE",
        "marketing-campaigns.md:246: ACTIVE when starts_at <= now < ends_at"
    );

    let room = Room::start().await;
    let id = room
        .seed_campaign("APPROVED", "-1 millisecond", "1 hour")
        .await;
    assert_eq!(room.fetch_campaign_status(id).await, "ACTIVE");
    room.finish().await;
}

/// §9 Row 3: APPROVED, `now < starts_at` -> `SCHEDULED`.
#[actix_web::test]
async fn approved_before_starts_at_is_scheduled() {
    let starts_at = t(9, 0, 0);
    let ends_at = t(17, 0, 0);
    let now = starts_at - chrono::Duration::nanoseconds(1);
    assert_eq!(
        derive_version_status("APPROVED", starts_at, ends_at, now),
        "SCHEDULED"
    );

    let room = Room::start().await;
    let id = room.seed_campaign("APPROVED", "10 seconds", "1 hour").await;
    assert_eq!(room.fetch_campaign_status(id).await, "SCHEDULED");
    room.finish().await;
}

/// §9 Row 4: APPROVED, `now` just before `ends_at` -> `ACTIVE`.
#[actix_web::test]
async fn approved_just_before_ends_at_is_active() {
    let starts_at = t(9, 0, 0);
    let ends_at = t(17, 0, 0);
    let now = ends_at - chrono::Duration::nanoseconds(1);
    assert_eq!(
        derive_version_status("APPROVED", starts_at, ends_at, now),
        "ACTIVE"
    );

    let room = Room::start().await;
    let id = room.seed_campaign("APPROVED", "-1 hour", "5 seconds").await;
    assert_eq!(room.fetch_campaign_status(id).await, "ACTIVE");
    room.finish().await;
}

/// §9 Row 5: PAUSED (any `now`) -> `PAUSED`.
#[actix_web::test]
async fn paused_at_any_now_is_paused() {
    let starts_at = t(9, 0, 0);
    let ends_at = t(17, 0, 0);
    for now in [
        starts_at - chrono::Duration::hours(1),
        starts_at,
        starts_at + chrono::Duration::hours(1),
        ends_at,
        ends_at + chrono::Duration::hours(1),
    ] {
        assert_eq!(
            derive_version_status("PAUSED", starts_at, ends_at, now),
            "PAUSED"
        );
    }

    let room = Room::start().await;
    let id = room
        .seed_campaign("PAUSED", "-1 hour", "-1 millisecond")
        .await;
    assert_eq!(room.fetch_campaign_status(id).await, "PAUSED");
    room.finish().await;
}
