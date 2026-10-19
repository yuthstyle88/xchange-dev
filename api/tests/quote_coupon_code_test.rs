//! Tests for `couponCode` on `POST /api/quotes` (MK-5 §7, §8).
//!
//! (a) an unknown or invalid coupon -> `couponInvalid` (§8), with no quote written;
//! (b) a valid coupon for a live campaign -> its benefit is in `benefits`;
//! (c) no `couponCode` -> identical to today.

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
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/quotes";
const FRONT: &str = "staff:FRONT:B-MAIN";

struct HeaderSessions;

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Actor::staff(
        format!("employee-{}", role.as_str()),
        role,
        BranchId::new(branch),
    )
}

fn sessions_of(spec: &str) -> Sessions {
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    if let Some(staff) = spec.strip_prefix("owner+") {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: staff_of(staff),
        };
    }
    Sessions {
        owner: None,
        staff: staff_of(spec),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
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

async fn post(db: &TestDb, who: &str, body: &Value, key: Option<&str>) -> Answer {
    common::install_test_pii_keys();
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(key) = key {
        req = req.insert_header(("idempotency-key", key));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn ask(db: &TestDb, who: &str, body: &Value) -> Answer {
    post(db, who, body, Some(&unique("key"))).await
}

#[allow(dead_code)]
struct SeededCampaign {
    campaign_id: Uuid,
    version_id: Uuid,
    rule_id: Uuid,
}

async fn seed_test_user(db: &TestDb) -> Uuid {
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'tester-coupon', 'ACTIVE', 'system') ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .bind(ORG_1)
    .execute(&db.pool)
    .await
    .expect("seed user");
    user_id
}

async fn seed_coupon_campaign(
    db: &TestDb,
    user_id: Uuid,
    branch: &str,
    budget_thb: &str,
    delta: &str,
) -> SeededCampaign {
    seed_coupon_campaign_in_group(db, user_id, branch, budget_thb, delta, "default", 1).await
}

async fn seed_coupon_campaign_in_group(
    db: &TestDb,
    user_id: Uuid,
    branch: &str,
    budget_thb: &str,
    delta: &str,
    stack_group: &str,
    priority: i32,
) -> SeededCampaign {
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();
    let budget = Decimal::from_str_exact(budget_thb).expect("budget decimal");

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
         $4, $5, $6, 'hash_coupon', now(), $7, now())",
    )
    .bind(version_id)
    .bind(ORG_1)
    .bind(campaign_id)
    .bind(budget)
    .bind(stack_group)
    .bind(priority)
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
    .bind(branch)
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

    let elig = json!({ "currencies": ["USD"], "requiresCoupon": true });
    let calc = json!({ "type": "FIXED_RATE", "delta": delta });

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, true, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(elig)
    .bind(calc)
    .execute(&db.pool)
    .await
    .expect("insert benefit_rule");

    SeededCampaign {
        campaign_id,
        version_id,
        rule_id,
    }
}

fn hasher() -> CouponCodeHasher {
    common::install_test_pii_keys();
    let keys = xc_customer::get_pii_keys().expect("test PII keys installed");
    CouponCodeHasher::new(&keys)
}

async fn insert_coupon(
    db: &TestDb,
    user_id: Uuid,
    rule_id: Uuid,
    code: &str,
    status: &str,
    customer_id: Option<&str>,
    expires_at: DateTime<Utc>,
) -> Uuid {
    let coupon_id = Uuid::new_v4();
    let code_hash = hasher().hash(code);

    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, issued_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, now())",
    )
    .bind(coupon_id)
    .bind(ORG_1)
    .bind(rule_id)
    .bind(&code_hash[..])
    .bind(status)
    .bind(customer_id)
    .bind(expires_at)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert coupon");

    coupon_id
}

/// (a) An unknown or invalid coupon -> couponInvalid (§8), with no quote written
#[tokio::test]
async fn unknown_or_invalid_coupon_refuses_validation_failed_and_writes_no_quote() {
    let db = TestDb::empty().await;
    db.migrate_through(None).await;
    db.branch("B-MAIN").await;
    db.publish("B-MAIN", &[Line::manual("USD", "1", "32.0000", "33.0000")])
        .await;
    db.customer("cust-1", "B-MAIN").await;
    let user_id = seed_test_user(&db).await;
    let _c = seed_coupon_campaign(&db, user_id, "B-MAIN", "5000.00", "0.50").await;

    // Subcase 1: Unknown coupon code
    let unknown_body = json!({
        "branch": "B-MAIN",
        "channel": "DESK",
        "customerId": "cust-1",
        "couponCode": "NONEXISTENT-CODE",
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });
    let ans = ask(&db, FRONT, &unknown_body).await;
    assert_eq!(
        ans.status,
        StatusCode::BAD_REQUEST,
        "unknown coupon must answer 400 Bad Request: {}",
        ans.text
    );
    assert_eq!(ans.code(), "couponInvalid");
    assert_eq!(
        ans.message(),
        "couponInvalid",
        "error message must mention couponInvalid: {}",
        ans.message()
    );
    assert_eq!(
        db.quote_rows().await,
        (0, 0),
        "no quote must be written on unknown coupon refusal"
    );

    // Subcase 2: Expired coupon
    let expired_code = "EXPIRED-COUPON";
    insert_coupon(
        &db,
        user_id,
        _c.rule_id,
        expired_code,
        "ISSUED",
        None,
        Utc::now() - chrono::Duration::hours(2),
    )
    .await;

    let expired_body = json!({
        "branch": "B-MAIN",
        "channel": "DESK",
        "customerId": "cust-1",
        "couponCode": expired_code,
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });
    let ans_exp = ask(&db, FRONT, &expired_body).await;
    assert_eq!(
        ans_exp.status,
        StatusCode::BAD_REQUEST,
        "expired coupon must answer 400 Bad Request"
    );
    assert_eq!(ans_exp.code(), "couponInvalid");
    assert_eq!(
        ans_exp.message(),
        "couponInvalid",
        "expired coupon must mention couponInvalid"
    );
    assert_eq!(
        db.quote_rows().await,
        (0, 0),
        "no quote must be written on expired coupon refusal"
    );

    // Subcase 3: Personal coupon for another customer
    db.customer("other-customer", "B-MAIN").await;
    let other_cust_code = "OTHER-CUST-COUPON";
    insert_coupon(
        &db,
        user_id,
        _c.rule_id,
        other_cust_code,
        "ISSUED",
        Some("other-customer"),
        Utc::now() + chrono::Duration::days(1),
    )
    .await;

    let mismatch_body = json!({
        "branch": "B-MAIN",
        "channel": "DESK",
        "customerId": "cust-1",
        "couponCode": other_cust_code,
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });
    let ans_mismatch = ask(&db, FRONT, &mismatch_body).await;
    assert_eq!(
        ans_mismatch.status,
        StatusCode::BAD_REQUEST,
        "personal coupon for another customer must answer 400 Bad Request"
    );
    assert_eq!(ans_mismatch.code(), "couponInvalid");
    assert_eq!(
        ans_mismatch.message(),
        "couponInvalid",
        "customer mismatch must mention couponInvalid"
    );
    assert_eq!(
        db.quote_rows().await,
        (0, 0),
        "no quote must be written on customer mismatch refusal"
    );
}

/// (b) A valid coupon for a live campaign -> its benefit is in `benefits`
#[tokio::test]
async fn valid_coupon_for_live_campaign_applies_benefit_and_reserves() {
    let db = TestDb::empty().await;
    db.migrate_through(None).await;
    db.branch("B-MAIN").await;
    db.publish("B-MAIN", &[Line::manual("USD", "1", "32.0000", "33.0000")])
        .await;
    db.customer("cust-1", "B-MAIN").await;
    let user_id = seed_test_user(&db).await;
    let c = seed_coupon_campaign(&db, user_id, "B-MAIN", "5000.00", "0.50").await;

    let coupon_code = "SAVE50";
    let coupon_id = insert_coupon(
        &db,
        user_id,
        c.rule_id,
        coupon_code,
        "ISSUED",
        None,
        Utc::now() + chrono::Duration::days(7),
    )
    .await;

    let body = json!({
        "branch": "B-MAIN",
        "channel": "DESK",
        "customerId": "cust-1",
        "couponCode": coupon_code,
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });

    let ans = ask(&db, FRONT, &body).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "valid coupon quote must succeed: {}",
        ans.text
    );

    // Benefit is in benefits
    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert_eq!(
        benefits.len(),
        1,
        "benefits must have exactly 1 applied benefit"
    );
    assert_eq!(
        benefits[0]["ruleId"],
        json!(c.rule_id.to_string()),
        "applied benefit ruleId must match campaign rule"
    );

    // Final rate improved: buy rate 32.0000 + delta 0.50 -> 32.5000
    let lines = ans.body["lines"].as_array().expect("lines array");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["baseRate"], "32.0000");
    assert_eq!(lines[0]["finalRate"], "32.5000");

    // Stored quote and reservation in DB
    let (quotes_count, lines_count) = db.quote_rows().await;
    assert_eq!(
        (quotes_count, lines_count),
        (1, 1),
        "quote must be stored in DB"
    );

    let quote_id = Uuid::parse_str(ans.body["id"].as_str().expect("quote id")).expect("valid uuid");

    // Check benefit_reservations row
    let res: Option<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT id, coupon_id, status FROM benefit_reservations WHERE quote_id = $1 AND rule_id = $2",
    )
    .bind(quote_id)
    .bind(c.rule_id)
    .fetch_optional(&db.pool)
    .await
    .expect("query reservation");

    let (_res_id, res_coupon_id, res_status) = res.expect("reservation row must exist");
    assert_eq!(res_status, "RESERVED");
    assert_eq!(
        res_coupon_id,
        Some(coupon_id),
        "reservation must link coupon_id"
    );
}

/// (c) No `couponCode` -> identical to today (empty benefits)
#[tokio::test]
async fn no_coupon_code_leaves_quote_unchanged_with_no_benefits() {
    let db = TestDb::empty().await;
    db.migrate_through(None).await;
    db.branch("B-MAIN").await;
    db.publish("B-MAIN", &[Line::manual("USD", "1", "32.0000", "33.0000")])
        .await;
    db.customer("cust-1", "B-MAIN").await;
    let user_id = seed_test_user(&db).await;
    // Campaign requires coupon
    let _c = seed_coupon_campaign(&db, user_id, "B-MAIN", "5000.00", "0.50").await;

    let body = json!({
        "branch": "B-MAIN",
        "channel": "DESK",
        "customerId": "cust-1",
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });

    let ans = ask(&db, FRONT, &body).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "quote without coupon must succeed: {}",
        ans.text
    );

    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert!(
        benefits.is_empty(),
        "benefits must be empty without coupon: {benefits:?}"
    );

    let lines = ans.body["lines"].as_array().expect("lines array");
    assert_eq!(lines[0]["baseRate"], "32.0000");
    assert_eq!(
        lines[0]["finalRate"], "32.0000",
        "final rate must equal base rate without coupon"
    );

    let (quotes_count, lines_count) = db.quote_rows().await;
    assert_eq!((quotes_count, lines_count), (1, 1));
}

/// (d) Coupon for campaign A with a competing coupon-gated campaign B -> only A applies,
/// and A's reservation holds the coupon id (MF1).
#[tokio::test]
async fn coupon_of_a_with_competing_coupon_rule_b_applies_only_a_and_reserves_coupon() {
    let db = TestDb::empty().await;
    db.migrate_through(None).await;
    db.branch("B-MAIN").await;
    db.publish("B-MAIN", &[Line::manual("USD", "1", "32.0000", "33.0000")])
        .await;
    db.customer("cust-1", "B-MAIN").await;
    let user_id = seed_test_user(&db).await;

    // Two requires_coupon campaigns on B-MAIN: A in group ga (+0.50, priority 1), B in group gb (+0.30, priority 2)
    let camp_a =
        seed_coupon_campaign_in_group(&db, user_id, "B-MAIN", "5000.00", "0.50", "ga", 1).await;
    let camp_b =
        seed_coupon_campaign_in_group(&db, user_id, "B-MAIN", "5000.00", "0.30", "gb", 2).await;

    // Only coupon for A exists
    let code_a = "ONLY-A";
    let coupon_a_id = insert_coupon(
        &db,
        user_id,
        camp_a.rule_id,
        code_a,
        "ISSUED",
        None,
        Utc::now() + chrono::Duration::days(7),
    )
    .await;

    let body = json!({
        "branch": "B-MAIN",
        "channel": "DESK",
        "customerId": "cust-1",
        "couponCode": code_a,
        "lines": [{
            "currency": "USD",
            "shopSide": "BUY",
            "quantity": "100"
        }]
    });

    let ans = ask(&db, FRONT, &body).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "quote with coupon A must succeed: {}",
        ans.text
    );

    // Only A's benefit is in benefits (not B)
    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert_eq!(
        benefits.len(),
        1,
        "only campaign A's benefit must be applied: {benefits:?}"
    );
    assert_eq!(
        benefits[0]["ruleId"],
        json!(camp_a.rule_id.to_string()),
        "applied benefit ruleId must match campaign A, not B"
    );

    // Final rate reflects A (+0.50), buy rate 32.0000 + 0.50 -> 32.5000 (not B's 32.3000)
    let lines = ans.body["lines"].as_array().expect("lines array");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["baseRate"], "32.0000");
    assert_eq!(lines[0]["finalRate"], "32.5000");

    let quote_id = Uuid::parse_str(ans.body["id"].as_str().expect("quote id")).expect("valid uuid");

    // Check benefit_reservations: exactly 1 reservation for quote_id, for A's rule, holding coupon_a_id
    let res: Option<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT id, coupon_id, status FROM benefit_reservations WHERE quote_id = $1 AND rule_id = $2",
    )
    .bind(quote_id)
    .bind(camp_a.rule_id)
    .fetch_optional(&db.pool)
    .await
    .expect("query reservation");

    let (_res_id, res_coupon_id, res_status) = res.expect("reservation row for A must exist");
    assert_eq!(res_status, "RESERVED");
    assert_eq!(
        res_coupon_id,
        Some(coupon_a_id),
        "reservation for A must hold coupon A's id"
    );

    // Ensure no reservation exists for B
    let res_b: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM benefit_reservations WHERE quote_id = $1 AND rule_id = $2")
            .bind(quote_id)
            .bind(camp_b.rule_id)
            .fetch_optional(&db.pool)
            .await
            .expect("query reservation B");
    assert!(
        res_b.is_none(),
        "no reservation must exist for competing coupon campaign B"
    );
}
