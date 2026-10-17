//! Integration route tests for FEE_WAIVER range validation (ticket #2795).
//!
//! Q1: quote 500 for a stored out-of-range rule, written with SQL as campaign_benefit_params_reader_test.rs does.
//! S1: PUT /api/campaigns/{id}/draft answers 400 validationFailed, field rules.calculation.percent, nothing written.
//! S2: the same for rules.calculation.amount.

mod common;
mod quote_common;

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
use quote_common::*;
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
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const USER: &str = "00000000-0000-0000-0000-000000000002";
const MAIN: &str = "MAIN";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const SEEDED_BUDGET: &str = "10000.00";
const SEEDED_CALC: &str = r#"{"delta":"0.11"}"#;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ────────────────────────────────────────────────────────────────────────────
// Q1: Quote 500 for stored out-of-range rule
// ────────────────────────────────────────────────────────────────────────────

const BRANCH: &str = "B-MAIN";

struct FrontDesk;

impl SessionReader for FrontDesk {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let signed_in = req.headers().get(AS).is_some();
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: if signed_in {
                    xc_auth::domain::policy::value_objects::actor::Actor::staff(
                        "employee-FRONT",
                        xc_auth::domain::policy::value_objects::role::Role::Front,
                        BranchId::new(BRANCH),
                    )
                } else {
                    None
                },
            }
        })
    }
}

struct QuoteAnswer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl QuoteAnswer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

async fn quote(db: &TestDb, fee_thb: &str) -> QuoteAnswer {
    common::install_test_pii_keys();
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(FrontDesk);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let body = json!({
        "branch": BRANCH,
        "channel": "DESK",
        "customerId": "cust-1",
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100",
            "feeThb": fee_thb
        }]
    });
    let req = atest::TestRequest::post()
        .uri("/api/quotes")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, "front"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    QuoteAnswer { status, text, body }
}

async fn room_with_fee_waiver_rule(calculation: Value) -> (TestDb, Uuid) {
    let db = TestDb::empty().await;
    db.migrate_through(None).await;
    db.branch(BRANCH).await;
    db.publish(BRANCH, &[Line::manual("USD", "1", "32.0000", "33.0000")])
        .await;
    db.customer("cust-1", BRANCH).await;

    let user_id = Uuid::new_v4();
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'tester-benefit-params', 'ACTIVE', 'system')",
    )
    .bind(user_id)
    .bind(ORG_1)
    .execute(&db.pool)
    .await
    .expect("seed user");

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign_id)
    .bind(ORG_1)
    .bind(unique("campaign"))
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert campaign");

    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
         5000.00, 'default', 1, 'hash_params', now(), $4, now())",
    )
    .bind(version_id)
    .bind(ORG_1)
    .bind(campaign_id)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(&db.pool)
        .await
        .expect("set current version");

    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version_id)
        .bind(BRANCH)
        .execute(&db.pool)
        .await
        .expect("insert branch link");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(&db.pool)
    .await
    .expect("insert budget");

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'FEE_WAIVER', 'PRICE', '{\"currencies\":[\"USD\"]}'::jsonb, $3, '{}'::jsonb, false, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(calculation)
    .execute(&db.pool)
    .await
    .expect("insert rule");

    (db, rule_id)
}

async fn reservations(db: &TestDb) -> i64 {
    db.count("SELECT count(*) FROM benefit_reservations").await
}

#[tokio::test]
async fn test_q1_quote_stored_out_of_range_fee_waiver_refused_500() {
    let (db, _rule) = room_with_fee_waiver_rule(json!({ "percent": "0" })).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "stored FEE_WAIVER with percent 0 must return 500 serverError: {}",
        ans.text
    );
    assert_eq!(ans.code(), "serverError", "{}", ans.text);
    assert_eq!(db.quote_rows().await, (0, 0), "no quote is written");
    assert_eq!(reservations(&db).await, 0, "no budget is reserved");

    db.finish().await;
}

// ────────────────────────────────────────────────────────────────────────────
// S1 & S2: Draft save refuses out-of-range percent and amount
// ────────────────────────────────────────────────────────────────────────────

fn marketing() -> Actor {
    Actor::new(
        "user-MARKETING",
        ORG,
        vec![ScopedRole::new(
            Role::Marketing,
            None::<BranchId>,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
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
        let is_marketing = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "MARKETING");
        let user = is_marketing.then(marketing);
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                staff_user: user.clone(),
                user,
            }
        })
    }
}

struct SaveAnswer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl SaveAnswer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> &str {
        self.body["error"]["field"].as_str().unwrap_or("")
    }
}

struct SaveRoom {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
}

impl SaveRoom {
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
        let schema = unique_id("fw_range_save");
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

        common::install_test_pii_keys();

        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
        }
    }

    async fn finish(self) {
        let SaveRoom { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

async fn save(room: &SaveRoom, campaign_id: Uuid, body: &Value) -> SaveAnswer {
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
        .method(Method::PUT)
        .uri(&format!("/api/campaigns/{campaign_id}/draft"))
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((AS, "MARKETING"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique_id("save")))
        .set_payload(body.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    SaveAnswer { status, text, body }
}

async fn seed_draft(pool: &PgPool) -> (Uuid, Uuid) {
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, NULL, $4::uuid, now())",
    )
    .bind(campaign_id)
    .bind(ORG)
    .bind(unique_id("campaign"))
    .bind(USER)
    .execute(pool)
    .await
    .expect("seed campaign");

    sqlx::query(
        "INSERT INTO campaign_versions ( \
             id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
             stack_group, priority, version_hash, created_by, created_at, edited_at \
         ) VALUES ($1, $2::uuid, $3, 1, 'DRAFT', now() - interval '1 hour', now() + interval '10 days', \
                   $4::numeric, 'RATE', 1, NULL, $5::uuid, now(), now())",
    )
    .bind(version_id)
    .bind(ORG)
    .bind(campaign_id)
    .bind(SEEDED_BUDGET)
    .bind(USER)
    .execute(pool)
    .await
    .expect("seed version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(pool)
        .await
        .expect("set current version");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
    )
    .bind(version_id)
    .execute(pool)
    .await
    .expect("seed budget");

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
             calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', '{\"currencies\":[\"USD\"]}'::jsonb, $3::jsonb, \
             '{}'::jsonb, false, 1)",
    )
    .bind(Uuid::new_v4())
    .bind(version_id)
    .bind(SEEDED_CALC)
    .execute(pool)
    .await
    .expect("seed rule");

    (campaign_id, version_id)
}

fn fee_waiver_rule(calculation: Value) -> Value {
    json!({
        "benefitType": "FEE_WAIVER",
        "eligibility": { "currencies": ["USD"] },
        "calculation": calculation,
        "caps": {},
        "requiresCoupon": false
    })
}

fn body_with(rules: Option<Vec<Value>>) -> Value {
    let mut body = json!({
        "startsAt": "2026-11-01T00:00:00Z",
        "endsAt": "2026-12-01T00:00:00Z",
        "budgetThb": "20000.00",
        "stackGroup": "RATE",
        "priority": 2,
        "branches": [MAIN]
    });
    if let Some(rules) = rules {
        body["rules"] = Value::Array(rules);
    }
    body
}

async fn stored_rules(pool: &PgPool, version_id: Uuid) -> Vec<(String, String, Value, i32)> {
    sqlx::query_as(
        "SELECT benefit_type, benefit_class, calculation, rule_no FROM benefit_rules \
         WHERE campaign_version_id = $1 ORDER BY rule_no",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read stored rules")
}

async fn stored_budget(pool: &PgPool, version_id: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT budget_thb FROM campaign_versions WHERE id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("read budget")
}

#[tokio::test]
async fn test_s1_draft_save_out_of_range_percent_refused() {
    let room = SaveRoom::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![fee_waiver_rule(json!({ "percent": "0" }))])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "percent 0 must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), "rules.calculation.percent", "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "refused save must not alter rules"
    );
    assert_eq!(
        stored_budget(&room.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "refused save must not alter budget"
    );

    room.finish().await;
}

#[tokio::test]
async fn test_s2_draft_save_out_of_range_amount_refused() {
    let room = SaveRoom::start().await;
    let (campaign_id, version_id) = seed_draft(&room.pool).await;

    let ans = save(
        &room,
        campaign_id,
        &body_with(Some(vec![fee_waiver_rule(json!({ "amount": "0" }))])),
    )
    .await;

    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "amount 0 must be refused with 400: {}",
        ans.text
    );
    assert_eq!(ans.code(), "validationFailed", "{}", ans.text);
    assert_eq!(ans.field(), "rules.calculation.amount", "{}", ans.text);
    assert_eq!(
        stored_rules(&room.pool, version_id).await,
        vec![(
            "RATE_IMPROVEMENT".to_owned(),
            "PRICE".to_owned(),
            serde_json::from_str::<Value>(SEEDED_CALC).expect("seed calc"),
            1
        )],
        "refused save must not alter rules"
    );
    assert_eq!(
        stored_budget(&room.pool, version_id).await,
        Decimal::from_str(SEEDED_BUDGET).expect("decimal"),
        "refused save must not alter budget"
    );

    room.finish().await;
}
