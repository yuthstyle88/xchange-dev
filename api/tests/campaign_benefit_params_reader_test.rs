//! Up-front tests: a stored benefit rule with malformed params is refused when applied (ticket B).
//!
//! Design `docs/design/blueprint/marketing-campaigns.md` §4.1 (`calculation` = "the calculator's
//! params, decimals as strings"), D-6 (`BenefitCalculator::apply(line, params)`, one file per kind).
//!
//! Written before the fix by the test writer; the implementer must not edit, delete or weaken them.
//!
//! Today `FeeWaiver::apply` and `RateImprovement::apply` read a malformed or missing param as 0
//! (`fee_waiver.rs:25,29,41,45,48`, `rate_improvement.rs:25,29,33,37,40`), so a quote silently
//! carries a 0 % / 0 THB / 0-rate benefit as if the campaign applied.
//!
//! Refusal `[proposed]`: the bad value is stored data, not the caller's input, so `POST /api/quotes`
//! answers **500 `serverError`** and writes no quote and no reservation. This is the same answer the
//! quote already gives for a stored rule whose `eligibility` or `caps` do not decode
//! (`pg_campaign_repository.rs` `AppError::Internal("failed to decode …")` → `quotes.rs` `serverError`).
//! A 400 would tell the teller their request was wrong; it was not.
//!
//! The rows are written with SQL on purpose: ticket A refuses them at save, and these tests pin the
//! reader for rows that bypassed it (older rows, direct writes).
//!
//! Behaviour rows (one test per reader site, so a restored fallback turns exactly one test red):
//! | stored `calculation` | site | answer |
//! |---|---|---|
//! | FEE_WAIVER `{"percent":"abc"}` | fee_waiver.rs:25 | 500 `serverError`, no quote, no reservation |
//! | FEE_WAIVER `{"percent":true}` | fee_waiver.rs:29 | same |
//! | FEE_WAIVER `{"amount":"abc"}` | fee_waiver.rs:41 | same |
//! | FEE_WAIVER `{"fixed":{}}` | fee_waiver.rs:45 | same |
//! | FEE_WAIVER `{"pct":"0.5"}` (no known key) | fee_waiver.rs:48-49 | same |
//! | RATE_IMPROVEMENT `{"delta":"abc"}` | rate_improvement.rs:25 | same |
//! | RATE_IMPROVEMENT `{"delta":true}` | rate_improvement.rs:29 | same |
//! | RATE_IMPROVEMENT `{"percent":"abc"}` | rate_improvement.rs:33 | same |
//! | RATE_IMPROVEMENT `{"percent":[]}` | rate_improvement.rs:37 | same |
//! | RATE_IMPROVEMENT `{"type":"FIXED_RATE"}` (no known key) | rate_improvement.rs:40-41 | same |
//! | each kind/key well formed | — | 200, the benefit applied with the right amount (keep-pin) |

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
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
                    Actor::staff("employee-FRONT", Role::Front, BranchId::new(BRANCH))
                } else {
                    None
                },
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
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

async fn quote(db: &TestDb, fee_thb: &str) -> Answer {
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

/// A branch with USD published at buy 32.0000 / sell 33.0000, a customer, and one APPROVED
/// campaign whose single PRICE rule (USD, no coupon) has `benefit_type` and `calculation` as given,
/// written straight into `benefit_rules`. Returns the rule id.
async fn room_with_rule(benefit_type: &str, calculation: Value) -> (TestDb, Uuid) {
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

async fn reservations(db: &TestDb) -> i64 {
    db.count("SELECT count(*) FROM benefit_reservations").await
}

/// The stored rule's params are malformed: the quote must be refused, not priced with a 0 benefit.
async fn refused(benefit_type: &str, calculation: Value) {
    let (db, _rule) = room_with_rule(benefit_type, calculation.clone()).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a stored {benefit_type} rule with calculation {calculation} must not be applied as 0: {}",
        ans.text
    );
    assert_eq!(ans.code(), "serverError", "{}", ans.text);
    assert_eq!(db.quote_rows().await, (0, 0), "no quote is written");
    assert_eq!(reservations(&db).await, 0, "no budget is reserved");
}

/// The stored rule's params are well formed: the quote applies exactly that benefit.
async fn applied(
    benefit_type: &str,
    calculation: Value,
    final_rate: &str,
    fee_thb: &str,
    amount_thb: &str,
) {
    let (db, rule_id) = room_with_rule(benefit_type, calculation.clone()).await;

    let ans = quote(&db, "100.00").await;

    assert_eq!(
        ans.status,
        StatusCode::OK,
        "{benefit_type} {calculation}: {}",
        ans.text
    );
    let line = &ans.body["lines"][0];
    assert_eq!(line["baseRate"], "32.0000", "{}", ans.text);
    assert_eq!(line["finalRate"], final_rate, "{}", ans.text);
    assert_eq!(line["feeThb"], fee_thb, "{}", ans.text);
    let benefits = ans.body["benefits"].as_array().expect("benefits array");
    assert_eq!(benefits.len(), 1, "{}", ans.text);
    assert_eq!(
        benefits[0]["ruleId"],
        json!(rule_id.to_string()),
        "{}",
        ans.text
    );
    let amount = benefits[0]["amountThb"]
        .as_str()
        .map(|s| s.parse::<Decimal>().expect("decimal amountThb"));
    assert_eq!(
        amount,
        Some(amount_thb.parse::<Decimal>().expect("decimal")),
        "{}",
        ans.text
    );
    assert_eq!(db.quote_rows().await, (1, 1));
    assert_eq!(reservations(&db).await, 1);
}

// ── FEE_WAIVER refusals ──

#[tokio::test]
async fn fee_waiver_percent_not_a_decimal_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "percent": "abc" })).await;
}

#[tokio::test]
async fn fee_waiver_percent_not_a_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "percent": true })).await;
}

#[tokio::test]
async fn fee_waiver_amount_not_a_decimal_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "amount": "abc" })).await;
}

#[tokio::test]
async fn fee_waiver_fixed_not_a_string_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "fixed": {} })).await;
}

#[tokio::test]
async fn fee_waiver_without_a_known_key_refuses_the_quote() {
    refused("FEE_WAIVER", json!({ "pct": "0.5" })).await;
}

// ── RATE_IMPROVEMENT refusals ──

#[tokio::test]
async fn rate_improvement_delta_not_a_decimal_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "delta": "abc" })).await;
}

#[tokio::test]
async fn rate_improvement_delta_not_a_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "delta": true })).await;
}

#[tokio::test]
async fn rate_improvement_percent_not_a_decimal_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "percent": "abc" })).await;
}

#[tokio::test]
async fn rate_improvement_percent_not_a_string_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "percent": [] })).await;
}

#[tokio::test]
async fn rate_improvement_without_a_known_key_refuses_the_quote() {
    refused("RATE_IMPROVEMENT", json!({ "type": "FIXED_RATE" })).await;
}

// ── keep-pins: well-formed params still apply the right amount ──

#[tokio::test]
async fn fee_waiver_percent_well_formed_waives_that_share_of_the_fee() {
    applied(
        "FEE_WAIVER",
        json!({ "percent": "0.5" }),
        "32.0000",
        "50.00",
        "50.00",
    )
    .await;
}

#[tokio::test]
async fn fee_waiver_amount_well_formed_waives_that_amount() {
    applied(
        "FEE_WAIVER",
        json!({ "amount": "30.00" }),
        "32.0000",
        "70.00",
        "30.00",
    )
    .await;
}

#[tokio::test]
async fn fee_waiver_fixed_well_formed_waives_that_amount() {
    applied(
        "FEE_WAIVER",
        json!({ "fixed": "25" }),
        "32.0000",
        "75.00",
        "25.00",
    )
    .await;
}

#[tokio::test]
async fn rate_improvement_delta_well_formed_moves_the_rate() {
    applied(
        "RATE_IMPROVEMENT",
        json!({ "type": "FIXED_RATE", "delta": "0.50" }),
        "32.5000",
        "100.00",
        "50.00",
    )
    .await;
}

#[tokio::test]
async fn rate_improvement_percent_well_formed_moves_the_rate_by_that_share() {
    applied(
        "RATE_IMPROVEMENT",
        json!({ "percent": "0.01" }),
        "32.3200",
        "100.00",
        "32.00",
    )
    .await;
}
