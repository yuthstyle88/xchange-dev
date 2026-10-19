//! Tests for #3475: `POST /api/quotes` answers `400 couponInvalid` with fixed message `"couponInvalid"`
//! for every coupon failure cause (unknown, expired/not ISSUED, another customer's), while keeping
//! valid coupons, no-coupon quotes, `409 rateUnavailable`, and other `400 validationFailed` domain
//! refusals unchanged.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role};
use xc_kernel::ids::BranchId;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes";
const BRANCH: &str = "B-MAIN";
const FRONT: &str = "staff:FRONT:B-MAIN";

struct HeaderSessions;

fn roles_of(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown role {other:?}"),
    }
}

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = parts.next().expect("a role");
    let branch = parts.next().expect("a branch");
    Some(test_callers::staff(
        &format!("employee-{role}"),
        roles_of(role),
        BranchId::new(branch),
    ))
}

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::nobody();
    }
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    if let Some(staff) = spec.strip_prefix("owner+") {
        return test_callers::callers(Some(test_callers::owner("owner-1")), staff_of(staff));
    }
    test_callers::callers(None, staff_of(spec))
}

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

async fn ask(db: &TestDb, body: &Value) -> Answer {
    common::install_test_pii_keys();
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, FRONT))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

struct SeededShop {
    db: TestDb,
    user_id: Uuid,
    rule_id: Uuid,
}

async fn setup_shop() -> SeededShop {
    let db = TestDb::empty().await;
    db.migrate_through(None).await;
    db.branch(BRANCH).await;
    db.publish(BRANCH, &[Line::manual("USD", "1", "32.0000", "33.0000")])
        .await;
    db.customer("cust-1", BRANCH).await;
    db.customer("other-customer", BRANCH).await;

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'tester-coupon-3475', 'ACTIVE', 'system') ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .bind(ORG_1)
    .execute(&db.pool)
    .await
    .expect("seed user");

    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();
    let budget = Decimal::from_str_exact("5000.00").expect("budget decimal");

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign_id)
    .bind(ORG_1)
    .bind(format!("campaign-{}", Uuid::new_v4()))
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert campaign");

    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
         $4, 'default', 1, 'hash_coupon_3475', now(), $5, now())",
    )
    .bind(version_id)
    .bind(ORG_1)
    .bind(campaign_id)
    .bind(budget)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(&db.pool)
        .await
        .expect("set current_version_id");

    sqlx::query(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(version_id)
    .bind(BRANCH)
    .execute(&db.pool)
    .await
    .expect("insert branch link");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
         VALUES ($1, 0, 0) ON CONFLICT (campaign_version_id) DO NOTHING",
    )
    .bind(version_id)
    .execute(&db.pool)
    .await
    .expect("insert campaign_budgets");

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, true, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(json!({ "currencies": ["USD"], "requiresCoupon": true }))
    .bind(json!({ "type": "FIXED_RATE", "delta": "0.50" }))
    .execute(&db.pool)
    .await
    .expect("insert benefit_rule");

    SeededShop {
        db,
        user_id,
        rule_id,
    }
}

fn hasher() -> CouponCodeHasher {
    common::install_test_pii_keys();
    let keys = xc_customer::get_pii_keys().expect("test PII keys installed");
    CouponCodeHasher::new(&keys)
}

async fn insert_coupon(
    shop: &SeededShop,
    code: &str,
    status: &str,
    customer_id: Option<&str>,
    expires_at: DateTime<Utc>,
) {
    let coupon_id = Uuid::new_v4();
    let code_hash = hasher().hash(code);

    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, issued_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, now())",
    )
    .bind(coupon_id)
    .bind(ORG_1)
    .bind(shop.rule_id)
    .bind(&code_hash[..])
    .bind(status)
    .bind(customer_id)
    .bind(expires_at)
    .bind(shop.user_id)
    .execute(&shop.db.pool)
    .await
    .expect("insert coupon");
}

fn quote_body_with_coupon(code: &str) -> Value {
    json!({
        "branch": BRANCH,
        "channel": "DESK",
        "customerId": "cust-1",
        "couponCode": code,
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    })
}

/// Row 1: Unknown coupon code -> 400, code: "couponInvalid", message: "couponInvalid", no quote row written.
#[tokio::test]
async fn unknown_coupon_code_answers_400_coupon_invalid_and_writes_no_quote() {
    let shop = setup_shop().await;

    let ans = ask(&shop.db, &quote_body_with_coupon("UNKNOWN-CODE")).await;
    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "body: {}", ans.text);
    assert_eq!(ans.code(), "couponInvalid", "body: {}", ans.text);
    assert_eq!(ans.message(), "couponInvalid", "body: {}", ans.text);
    assert_eq!(shop.db.quote_rows().await, (0, 0));

    shop.db.finish().await;
}

/// Row 2: Coupon expired, or status not ISSUED -> the same answer, byte for byte.
#[tokio::test]
async fn expired_or_non_issued_coupon_answers_identical_coupon_invalid_byte_for_byte() {
    let shop = setup_shop().await;

    insert_coupon(
        &shop,
        "EXPIRED-CODE",
        "ISSUED",
        None,
        Utc::now() - chrono::Duration::hours(2),
    )
    .await;
    insert_coupon(
        &shop,
        "REDEEMED-CODE",
        "REDEEMED",
        None,
        Utc::now() + chrono::Duration::days(7),
    )
    .await;

    let ans_unknown = ask(&shop.db, &quote_body_with_coupon("UNKNOWN-CODE")).await;
    let ans_expired = ask(&shop.db, &quote_body_with_coupon("EXPIRED-CODE")).await;
    let ans_redeemed = ask(&shop.db, &quote_body_with_coupon("REDEEMED-CODE")).await;

    assert_eq!(ans_expired.status, StatusCode::BAD_REQUEST);
    assert_eq!(ans_expired.code(), "couponInvalid");
    assert_eq!(ans_expired.message(), "couponInvalid");
    assert_eq!(ans_expired.text, ans_unknown.text);

    assert_eq!(ans_redeemed.status, StatusCode::BAD_REQUEST);
    assert_eq!(ans_redeemed.code(), "couponInvalid");
    assert_eq!(ans_redeemed.message(), "couponInvalid");
    assert_eq!(ans_redeemed.text, ans_unknown.text);

    assert_eq!(shop.db.quote_rows().await, (0, 0));

    shop.db.finish().await;
}

/// Row 3: A personal coupon given for another customer -> the same answer, byte for byte (no "mismatch" text).
#[tokio::test]
async fn personal_coupon_for_another_customer_answers_identical_coupon_invalid_with_no_mismatch_text()
 {
    let shop = setup_shop().await;

    insert_coupon(
        &shop,
        "OTHER-CUST-CODE",
        "ISSUED",
        Some("other-customer"),
        Utc::now() + chrono::Duration::days(7),
    )
    .await;

    let ans_unknown = ask(&shop.db, &quote_body_with_coupon("UNKNOWN-CODE")).await;
    let ans_mismatch = ask(&shop.db, &quote_body_with_coupon("OTHER-CUST-CODE")).await;

    assert_eq!(ans_mismatch.status, StatusCode::BAD_REQUEST);
    assert_eq!(ans_mismatch.code(), "couponInvalid");
    assert_eq!(ans_mismatch.message(), "couponInvalid");
    assert!(!ans_mismatch.text.to_lowercase().contains("mismatch"));
    assert_eq!(ans_mismatch.text, ans_unknown.text);
    assert_eq!(shop.db.quote_rows().await, (0, 0));

    shop.db.finish().await;
}

/// Row 4: A valid coupon -> 200, the quote with its benefit, unchanged.
#[tokio::test]
async fn valid_coupon_answers_200_with_its_benefit_unchanged() {
    let shop = setup_shop().await;

    insert_coupon(
        &shop,
        "VALID-CODE",
        "ISSUED",
        Some("cust-1"),
        Utc::now() + chrono::Duration::days(7),
    )
    .await;

    let ans = ask(&shop.db, &quote_body_with_coupon("VALID-CODE")).await;
    assert_eq!(ans.status, StatusCode::OK, "body: {}", ans.text);
    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert_eq!(benefits.len(), 1);
    assert_eq!(benefits[0]["ruleId"], json!(shop.rule_id.to_string()));
    assert_eq!(ans.body["lines"][0]["finalRate"], "32.5000");
    assert_eq!(shop.db.quote_rows().await, (1, 1));

    shop.db.finish().await;
}

/// Row 5: No `couponCode` -> unchanged.
#[tokio::test]
async fn no_coupon_code_answers_200_unchanged() {
    let shop = setup_shop().await;

    let body = json!({
        "branch": BRANCH,
        "channel": "DESK",
        "customerId": "cust-1",
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });
    let ans = ask(&shop.db, &body).await;
    assert_eq!(ans.status, StatusCode::OK, "body: {}", ans.text);
    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert!(benefits.is_empty());
    assert_eq!(ans.body["lines"][0]["finalRate"], "32.0000");
    assert_eq!(shop.db.quote_rows().await, (1, 1));

    shop.db.finish().await;
}

/// Row 6: Rate unavailable -> 409 rateUnavailable, unchanged.
#[tokio::test]
async fn rate_unavailable_answers_409_rate_unavailable_unchanged() {
    let shop = setup_shop().await;

    let body = json!({
        "branch": BRANCH,
        "channel": "DESK",
        "customerId": "cust-1",
        "lines": [{
            "currency": "JPY",
            "shopSide": "BUY",
            "quantity": "1000"
        }]
    });
    let ans = ask(&shop.db, &body).await;
    assert_eq!(ans.status, StatusCode::CONFLICT, "body: {}", ans.text);
    assert_eq!(ans.code(), "rateUnavailable", "body: {}", ans.text);
    assert_eq!(shop.db.quote_rows().await, (0, 0));

    shop.db.finish().await;
}

/// Row 7: Another Domain refusal (e.g. fee exceeds THB amount) -> 400 validationFailed with its own message, unchanged.
#[tokio::test]
async fn other_domain_refusal_answers_400_validation_failed_with_its_own_message_unchanged() {
    let shop = setup_shop().await;

    let body = json!({
        "branch": BRANCH,
        "channel": "DESK",
        "customerId": "cust-1",
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100",
            "feeThb": "5000.00"
        }]
    });
    let ans = ask(&shop.db, &body).await;
    assert_eq!(ans.status, StatusCode::BAD_REQUEST, "body: {}", ans.text);
    assert_eq!(ans.code(), "validationFailed", "body: {}", ans.text);
    assert_eq!(
        ans.message(),
        "validationFailed: fee exceeds thb amount",
        "body: {}",
        ans.text
    );
    assert_eq!(shop.db.quote_rows().await, (0, 0));

    shop.db.finish().await;
}
