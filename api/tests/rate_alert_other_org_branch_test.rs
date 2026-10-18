//! Ticket #2500: `POST /api/app/rate-alerts` (`rateAlertCreate`) naming another organisation's branch answers 400
//! `validationFailed` with nothing written, identical to a non-publishing or unknown branch.

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
use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_alert::application::rate_alert_service::{
    CreateRateAlert, RateAlertError, RateAlertService,
};
use xc_alert::domain::CustomerSide;
use xc_alert::domain::condition::AlertCondition;
use xc_alert::infrastructure::repository::PgRateAlertRepository;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::infrastructure::currency::currency_register::PgCurrencyRegister;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const CREATE: &str = "/api/app/rate-alerts";
const MAIN: &str = "MAIN";
const NORTH: &str = "NORTH";
const ORGB_B01: &str = "ORGB-B01";
const NO_SUCH_BRANCH: &str = "NO-SUCH-BRANCH";
/// The seeded organisation (`20261011100000_organizations.sql`).
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ORG_B: &str = "00000000-0000-0000-0000-000000000002";
const CX1: &str = "CX-RA2B-1";

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
}

impl Room {
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
        let schema = unique("ra2500");
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
        let room = Self { pool, schema };
        room.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN'), ('NORTH', 'NORTH') ON CONFLICT (id) DO NOTHING",
        )
        .await;
        room.exec(&format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{ORG_B}', 'ORGB', NULL, 'Asia/Bangkok', now())"
        ))
        .await;
        room.exec(&format!(
            "INSERT INTO branches (id, organization_id, code, name) \
             VALUES ('{ORGB_B01}', '{ORG_B}', '{ORGB_B01}', '{ORGB_B01}')"
        ))
        .await;
        room.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) VALUES \
             ('{CX1}', '{ORG}', '{CX1}', 'Somchai', 'LIVE', now())"
        ))
        .await;
        room.publish_usd_in(ORG, MAIN).await;
        room.settings(10).await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn publish_usd_in(&self, org: &str, branch: &str) {
        let approval: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), $1::uuid, 'RATE_PUBLISH', $2, $3, 'h', 'p', '{}', 'test', now(), 'APPROVED', $3) \
             RETURNING id",
        )
        .bind(org)
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("an approved request for the publication");
        let publication: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1::uuid, $2, 1, now(), $3, 'test') RETURNING id",
        )
        .bind(org)
        .bind(branch)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("the publication");
        sqlx::query(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, unit_size, buy_rate, sell_rate, \
             source_snapshot) VALUES (gen_random_uuid(), $1::uuid, $2, 'USD', 1, 33.50, 34.00, $3)",
        )
        .bind(org)
        .bind(publication)
        .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
        .execute(&self.pool)
        .await
        .expect("the USD line");
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("make it current");
    }

    async fn settings(&self, max: i32) {
        let by = self.user().await;
        sqlx::query(
            "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, \
             default_lifetime_days, observe_policy, updated_by, updated_at) \
             VALUES ($1::uuid, 86400, $2, 30, 'ALL_FRESH', $3::uuid, now())",
        )
        .bind(ORG)
        .bind(max)
        .bind(&by)
        .execute(&self.pool)
        .await
        .expect("write the settings row");
    }

    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("u"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    async fn customer(&self, customer_id: &str) -> Who {
        let id = self.user().await;
        Who(format!("{id}|CUSTOMER|{customer_id}"))
    }

    fn service(&self) -> RateAlertService {
        RateAlertService::new(
            Arc::new(PgRateAlertRepository::new(self.pool.clone())),
            Arc::new(PgCurrencyRegister::new(self.pool.clone())),
            Arc::new(PgPublishedRates::new(self.pool.clone())),
        )
    }

    async fn alerts(&self) -> i64 {
        self.count("SELECT count(*) FROM rate_alerts").await
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct Who(String);

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, customer) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        (!customer.is_empty()).then_some(customer),
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
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
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
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
        self.body["error"]["code"].as_str().unwrap_or("").to_owned()
    }
}

async fn create(room: &Room, who: &Who, branch: &str, key: &str) -> Answer {
    let body = json!({
        "branch": branch,
        "currency": "USD",
        "customerSide": "CUSTOMER_BUY",
        "condition": "ABOVE",
        "target": "44.10",
    });
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(CREATE)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.0.clone()))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key.to_owned()))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn expected_non_publishing_body(branch: &str) -> Value {
    json!({
        "error": {
            "code": "validationFailed",
            "message": format!("branch {branch} does not publish USD Sell"),
        }
    })
}

/// O1: `ORGB-B01`, organisation B's branch with a current USD publication → 400 `validationFailed`; 0 `rate_alerts` rows.
#[actix_web::test]
async fn o1_other_org_branch_with_publication_answers_400_validation_failed_and_writes_nothing() {
    let room = Room::start().await;
    room.publish_usd_in(ORG_B, ORGB_B01).await;
    let cx1 = room.customer(CX1).await;
    let a = create(&room, &cx1, ORGB_B01, &unique("key")).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.body, expected_non_publishing_body(ORGB_B01), "{}", a.text);
    assert_eq!(room.alerts().await, 0);
    room.finish().await;
}

/// O2: `ORGB-B01` with no publication → 400 `validationFailed`, the same answer as O1.
#[actix_web::test]
async fn o2_other_org_branch_without_publication_answers_same_400_validation_failed_as_o1() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;
    let a = create(&room, &cx1, ORGB_B01, &unique("key")).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.body, expected_non_publishing_body(ORGB_B01), "{}", a.text);
    assert_eq!(room.alerts().await, 0);
    room.finish().await;
}

/// O3: `NO-SUCH-BRANCH` → 400 `validationFailed` (keep: today's non-publishing refusal).
#[actix_web::test]
async fn o3_unknown_or_non_publishing_branch_is_refused_validation_failed() {
    let room = Room::start().await;
    let svc = room.service();
    let mut tx = room.pool.begin().await.expect("begin tx");
    let err = svc
        .create(
            &mut tx,
            CX1,
            &CreateRateAlert {
                branch: BranchId::new(NO_SUCH_BRANCH),
                currency: CurrencyCode::parse("USD").expect("USD"),
                customer_side: CustomerSide::CustomerBuy,
                condition: AlertCondition::Above,
                target: Decimal::from_str("44.10").expect("decimal"),
                expires_at: None,
            },
            &unique("key"),
            Utc::now(),
        )
        .await
        .expect_err("NO-SUCH-BRANCH must be refused");
    tx.rollback().await.expect("rollback");
    match err {
        RateAlertError::ValidationFailed { reason, message } => {
            assert_eq!(reason, None);
            assert_eq!(
                message,
                format!("branch {NO_SUCH_BRANCH} does not publish USD Sell")
            );
        }
        other => panic!("expected ValidationFailed, got {other:?}"),
    }

    let cx1 = room.customer(CX1).await;
    let a = create(&room, &cx1, NORTH, &unique("key")).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{}", a.text);
    assert_eq!(a.code(), "validationFailed", "{}", a.text);
    assert_eq!(a.body, expected_non_publishing_body(NORTH), "{}", a.text);
    assert_eq!(room.alerts().await, 0);
    room.finish().await;
}

/// O4: `MAIN` (organisation A, publishes USD) → 200, alert created.
#[actix_web::test]
async fn o4_own_org_publishing_branch_creates_alert() {
    let room = Room::start().await;
    let cx1 = room.customer(CX1).await;
    let a = create(&room, &cx1, MAIN, &unique("key")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert!(a.body["id"].as_str().is_some(), "{}", a.text);
    assert_eq!(a.body["version"], json!(1), "{}", a.text);
    assert_eq!(room.alerts().await, 1);
    room.finish().await;
}
