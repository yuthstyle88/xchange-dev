//! Shared test setup for campaign benefit-rule route tests (#2882).

#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor as PolicyActor;
use xc_auth::domain::policy::value_objects::role::Role as PolicyRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

use crate::common;
use crate::quote_common::*;

pub const AS: &str = "x-test-as";
pub const HOST: &str = "xc.test";
pub const SAME_ORIGIN: &str = "http://xc.test";
pub const BRANCH: &str = "B-MAIN";
pub const MAIN: &str = "MAIN";
pub const USER: &str = "00000000-0000-0000-0000-000000000002";
pub const SEEDED_BUDGET: &str = "10000.00";
pub const SEEDED_CALC: &str = r#"{"delta":"0.11"}"#;

pub struct FrontDesk;

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
                    PolicyActor::staff("employee-FRONT", PolicyRole::Front, BranchId::new(BRANCH))
                } else {
                    None
                },
            }
        })
    }
}

pub fn marketing() -> AccessActor {
    AccessActor::new(
        "user-MARKETING",
        ORG_1,
        vec![ScopedRole::new(
            AccessRole::Marketing,
            None::<BranchId>,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

pub struct MarketingCallers;

impl SessionReader for MarketingCallers {
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
                staff_user: user.clone(),
                user,
            }
        })
    }
}

pub struct Answer {
    pub status: StatusCode,
    pub text: String,
    pub body: Value,
}

impl Answer {
    pub fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    pub fn field(&self) -> &str {
        self.body["error"]["field"].as_str().unwrap_or("")
    }

    pub fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

pub type QuoteAnswer = Answer;
pub type SaveAnswer = Answer;
pub type HeaderCallers = MarketingCallers;
pub const ORG: &str = ORG_1;

pub async fn save(db: &TestDb, campaign_id: Uuid, body: &Value) -> Answer {
    save_draft(db, campaign_id, body).await
}

pub async fn quote(db: &TestDb, fee_thb: &str) -> Answer {
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
    Answer { status, text, body }
}

pub async fn save_draft(db: &TestDb, campaign_id: Uuid, body: &Value) -> Answer {
    common::install_test_pii_keys();
    let guard = Arc::new(AppSessionGuard::new(20));
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(MarketingCallers);
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
        .insert_header(("idempotency-key", unique("save")))
        .set_payload(body.to_string());

    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A branch with USD published at buy 32.0000 / sell 33.0000, a customer, and one APPROVED
/// campaign whose single PRICE rule (USD, no coupon) has `benefit_type` and `calculation` as given,
/// written straight into `benefit_rules`. Returns the rule id.
pub async fn room_with_rule(benefit_type: &str, calculation: Value) -> (TestDb, Uuid) {
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
         VALUES ($1, $2, $3, 'PRICE', '{\"currencies\":[\"USD\"]}'::jsonb, $4, '{}'::jsonb, false, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(benefit_type)
    .bind(calculation)
    .execute(&db.pool)
    .await
    .expect("insert rule");

    (db, rule_id)
}

pub async fn reservations(db: &TestDb) -> i64 {
    db.count("SELECT count(*) FROM benefit_reservations").await
}

/// A DRAFT campaign on MAIN with one stored, well-formed rule (`rule_no` 1, `SEEDED_CALC`).
pub async fn seed_draft(pool: &PgPool) -> (Uuid, Uuid) {
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, version, created_at, created_by) \
         VALUES ($1::uuid, $2::uuid, 'Test User', 'ACTIVE', 1, now(), 'test') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(USER)
    .bind(ORG_1)
    .execute(pool)
    .await
    .expect("seed test user");

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, NULL, $4::uuid, now())",
    )
    .bind(campaign_id)
    .bind(ORG_1)
    .bind(unique("campaign"))
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
    .bind(ORG_1)
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

pub fn rule(benefit_type: &str, calculation: Value) -> Value {
    json!({
        "benefitType": benefit_type,
        "eligibility": { "currencies": ["USD"] },
        "calculation": calculation,
        "caps": {},
        "requiresCoupon": false
    })
}

pub fn body_with(rules: Option<Vec<Value>>) -> Value {
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

pub async fn stored_rules(pool: &PgPool, version_id: Uuid) -> Vec<(String, String, Value, i32)> {
    sqlx::query_as(
        "SELECT benefit_type, benefit_class, calculation, rule_no FROM benefit_rules \
         WHERE campaign_version_id = $1 ORDER BY rule_no",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
    .expect("read stored rules")
}

pub async fn stored_budget(pool: &PgPool, version_id: Uuid) -> Decimal {
    sqlx::query_scalar("SELECT budget_thb FROM campaign_versions WHERE id = $1")
        .bind(version_id)
        .fetch_one(pool)
        .await
        .expect("read budget")
}
