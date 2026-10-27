// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE under the xchange BE lane controller claude/bc8fd29a). Do not edit, delete or weaken.

//! Up-front tests for BE-4: `POST /api/v3/compute/planner_preview` answers `benefit`, the customer's indicative
//! benefit on the plan (design `marketing-campaigns.md` §6.4; O-3 A decided on #564 issuecomment-6262478258;
//! OD-1 B, OD-2 A, OD-3 A decided by the owner 2026-10-27).
//!
//! Rows of the behaviour table §6.4.5 covered here, one test each:
//! - row 1 (`be4_row1_*`): an anonymous call with one ACTIVE `RATE_IMPROVEMENT` rule with no customer condition and
//!   no `channels` answers `ESTIMATED`, `INDICATIVE`, `APP`, one item, `finalRate ≠ rate`, and the same `thb` as
//!   today's answer for the same plan (E-1, E-7, §6.4.3);
//! - row 2 (`be4_row2_*`): the same call writes nothing: `quotes`, `quote_lines`, `benefit_reservations` and the
//!   sum of `campaign_budgets.consumed_thb` are unchanged (O-3: never reserves, E-4);
//! - row 8 (`be4_row8_*`): a signed-in customer of another organisation is evaluated customer-less: a segment rule
//!   (which that customer would pass if it were used) answers `NONE`, and a rule with no customer condition still
//!   answers `ESTIMATED` (E-5);
//! - row 10 (`be4_row10_*`): a stale line its policy refuses refuses the whole preview with `rateUnavailable` and
//!   no `benefit`, as today; a stale line its policy admits answers `ESTIMATED` and `stale: true` (E-7).
//!
//! Harness: `quote_common` (a PostgreSQL schema per test, read-only use) plus a branch id of its own per test.
//! Callers are injected through `SessionReader`, as `c05_my_benefits_test.rs` does. Needs `TEST_DATABASE_URL`
//! naming a database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration, Utc};
use quote_common::{Line, ORG_1, TestDb, unique};
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS_HEADER: &str = "x-test-as";
const ROUTE: &str = "/api/v3/compute/planner_preview";
const ORG_2: &str = "00000000-0000-0000-0000-0000000000b4";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

/// USD 1 at 32.00 (shop buys) / 33.00 (shop sells). `CUSTOMER_SELL` is the shop's BUY (P4), so the rule's
/// `FIXED_RATE` +0.50 improves 32.00 to 32.50.
const USD: Line = Line::manual("USD", "1", "32.00", "33.00");

// ── Callers ─────────────────────────────────────────────────────────────────

fn customer_of(org: &str, customer: &str) -> Actor {
    Actor::new(
        &format!("user-{customer}"),
        org,
        vec![ScopedRole::new(
            Role::Customer,
            None,
            Utc::now() - Duration::hours(1),
            None,
        )],
        Some(customer),
    )
}

/// `""` = anonymous; `org2:<customer>` = a signed-in customer of `ORG_2`.
fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return Callers {
            user: None,
            staff_user: None,
        };
    }
    if let Some(customer) = spec.strip_prefix("org2:") {
        return Callers {
            user: Some(customer_of(ORG_2, customer)),
            staff_user: None,
        };
    }
    panic!("unknown test caller spec: {spec}");
}

struct TestSessions;

impl SessionReader for TestSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move { callers_of(&spec) })
    }
}

// ── HTTP ────────────────────────────────────────────────────────────────────

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// `planner_preview` of `CUSTOMER_SELL` `quantity` USD at `branch`, as `who`.
async fn preview(db: &TestDb, who: &str, branch: &str, quantity: &str) -> Answer {
    common::install_test_pii_keys();
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(TestSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let body = json!({
        "branch": branch,
        "currency": "USD",
        "customerSide": "CUSTOMER_SELL",
        "amount": { "quantity": quantity },
    });
    let req = atest::TestRequest::post()
        .uri(ROUTE)
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", unique("be4-key")))
        .insert_header((AS_HEADER, who))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A 200 answer, or a panic that prints the body.
async fn preview_ok(db: &TestDb, who: &str, branch: &str, quantity: &str) -> Answer {
    let a = preview(db, who, branch, quantity).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "planner_preview at {branch} answers 200: {}",
        a.text
    );
    a
}

// ── Seeding ─────────────────────────────────────────────────────────────────

/// A branch id of its own (`A-Za-z0-9-` only, `BranchId::is_well_formed`).
fn branch_id(prefix: &str) -> String {
    unique(prefix).replace('_', "-").to_uppercase()
}

/// A branch of `ORG_1` of its own, with USD published.
async fn branch_with_usd(db: &TestDb, prefix: &str) -> String {
    let branch = branch_id(prefix);
    db.branch(&branch).await;
    db.publish(&branch, &[USD]).await;
    branch
}

async fn user(db: &TestDb) -> Uuid {
    let id = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{id}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
        unique("be4-user")
    ))
    .await;
    id
}

/// An APPROVED, running campaign of `ORG_1` on `branch` (ends in 30 days, budget 5,000 THB untouched) with one
/// `RATE_IMPROVEMENT` rule (`FIXED_RATE` +0.50) of `eligibility`, no coupon. Answers the rule id.
async fn campaign(db: &TestDb, branch: &str, eligibility: Value, segment: Option<Uuid>) -> Uuid {
    let by = user(db).await;
    let (campaign, version, rule) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign)
    .bind(ORG_1)
    .bind(unique("be4-campaign"))
    .bind(by)
    .execute(&db.pool)
    .await
    .expect("insert campaign");
    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, segment_snapshot_id, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
             5000.00, 'default', 1, $4, 'hash-be4', now(), $5, now())",
    )
    .bind(version)
    .bind(ORG_1)
    .bind(campaign)
    .bind(segment)
    .bind(by)
    .execute(&db.pool)
    .await
    .expect("insert campaign version");
    db.exec(&format!(
        "UPDATE campaigns SET current_version_id = '{version}' WHERE id = '{campaign}'"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ('{version}', '{branch}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ('{version}', 0, 0)"
    ))
    .await;
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, \
         caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, false, 1)",
    )
    .bind(rule)
    .bind(version)
    .bind(eligibility)
    .bind(json!({ "type": "FIXED_RATE", "delta": "0.50" }))
    .execute(&db.pool)
    .await
    .expect("insert benefit_rules");
    rule
}

/// A segment snapshot of `ORG_1` whose only member is `customer`; answers the snapshot id.
async fn snapshot_of(db: &TestDb, customer: &str) -> Uuid {
    let by = user(db).await;
    let (definition, snapshot) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, '{}'::jsonb, $4, now())",
    )
    .bind(definition)
    .bind(ORG_1)
    .bind(format!("seg-{definition}"))
    .bind(by)
    .execute(&db.pool)
    .await
    .expect("insert segment definition");
    sqlx::query(
        "INSERT INTO segment_snapshots (id, organization_id, definition_id, as_of, member_count, taken_by) \
         VALUES ($1, $2::uuid, $3, now(), 1, $4)",
    )
    .bind(snapshot)
    .bind(ORG_1)
    .bind(definition)
    .bind(by)
    .execute(&db.pool)
    .await
    .expect("insert segment snapshot");
    sqlx::query(
        "INSERT INTO segment_snapshot_members (snapshot_id, customer_id, metrics) VALUES ($1, $2, '{}'::jsonb)",
    )
    .bind(snapshot)
    .bind(customer)
    .execute(&db.pool)
    .await
    .expect("insert snapshot member");
    snapshot
}

/// Granted MARKETING consent for `customer`, recorded under `org`.
async fn consent(db: &TestDb, org: &str, customer: &str) {
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, \
             effective_at, evidence_ref) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', true, now() - interval '1 hour', 'test-be4')",
    )
    .bind(Uuid::new_v4())
    .bind(org)
    .bind(customer)
    .execute(&db.pool)
    .await
    .expect("insert consent");
}

// ── Assertions ──────────────────────────────────────────────────────────────

fn s<'a>(v: &'a Value, key: &str, text: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("`{key}` is a string: {text}"))
}

/// Every object key anywhere under `v`.
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                out.push(k.clone());
                keys(child, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| keys(c, out)),
        _ => {}
    }
}

/// The `ESTIMATED` shape of §6.4.3 with `items` of length `n`; answers the benefit.
fn assert_estimated(a: &Answer, n: usize) -> &Value {
    let b = &a.body["benefit"];
    assert_eq!(b["state"], "ESTIMATED", "an estimate: {}", a.text);
    assert_eq!(b["basis"], "INDICATIVE", "labelled indicative: {}", a.text);
    assert_eq!(b["channel"], "APP", "judged on APP (E-5): {}", a.text);
    for key in [
        "evaluatedAt",
        "validUntil",
        "publicationId",
        "finalRate",
        "thbAfter",
        "benefitThb",
    ] {
        assert!(
            !s(b, key, &a.text).is_empty(),
            "`benefit.{key}` is present: {}",
            a.text
        );
    }
    let items = b["items"]
        .as_array()
        .unwrap_or_else(|| panic!("`benefit.items` is a list: {}", a.text));
    assert_eq!(items.len(), n, "{n} item(s): {}", a.text);
    for item in items {
        for key in [
            "ruleId",
            "label",
            "benefitType",
            "amountThb",
            "rateAfter",
            "endsAt",
        ] {
            assert!(
                !s(item, key, &a.text).is_empty(),
                "item `{key}` is present: {}",
                a.text
            );
        }
    }
    let mut all = Vec::new();
    keys(b, &mut all);
    for banned in ["couponId", "reservationId", "reason"] {
        assert!(
            !all.iter().any(|k| k == banned),
            "an ESTIMATED benefit carries no `{banned}`: {}",
            a.text
        );
    }
    b
}

// ── Row 1 ───────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn be4_row1_an_anonymous_preview_estimates_an_open_rate_improvement_and_keeps_todays_thb() {
    let db = TestDb::start().await;
    let branch = branch_with_usd(&db, "be4r1").await;

    // Today's answer for the plan, with no campaign at all.
    let before = preview_ok(&db, "", &branch, "100").await;
    let todays_thb = s(&before.body, "thb", &before.text).to_owned();
    let todays_rate = s(&before.body, "rate", &before.text).to_owned();

    let rule = campaign(&db, &branch, json!({}), None).await;
    let a = preview_ok(&db, "", &branch, "100").await;

    assert_eq!(
        s(&a.body, "thb", &a.text),
        todays_thb,
        "`thb` stays the published amount (E-1): {}",
        a.text
    );
    assert_eq!(
        s(&a.body, "rate", &a.text),
        todays_rate,
        "`rate` stays the published rate (E-1): {}",
        a.text
    );
    let b = assert_estimated(&a, 1);
    assert_ne!(
        s(b, "finalRate", &a.text),
        s(&a.body, "rate", &a.text),
        "the rule improves the rate: {}",
        a.text
    );
    assert_eq!(
        s(&b["items"][0], "ruleId", &a.text),
        rule.to_string(),
        "the item is the seeded rule: {}",
        a.text
    );
    assert_eq!(
        s(&b["items"][0], "benefitType", &a.text),
        "RATE_IMPROVEMENT",
        "{}",
        a.text
    );
    db.finish().await;
}

// ── Row 2 ───────────────────────────────────────────────────────────────────

async fn footprint(db: &TestDb) -> (i64, i64, i64, String) {
    let consumed = db
        .text("SELECT COALESCE(sum(consumed_thb), 0)::text FROM campaign_budgets")
        .await
        .expect("a sum");
    (
        db.count("SELECT count(*) FROM quotes").await,
        db.count("SELECT count(*) FROM quote_lines").await,
        db.count("SELECT count(*) FROM benefit_reservations").await,
        consumed,
    )
}

#[actix_web::test]
async fn be4_row2_the_estimate_writes_no_quote_and_reserves_no_budget() {
    let db = TestDb::start().await;
    let branch = branch_with_usd(&db, "be4r2").await;
    campaign(&db, &branch, json!({}), None).await;

    let before = footprint(&db).await;
    let a = preview_ok(&db, "", &branch, "100").await;
    let after = footprint(&db).await;

    assert_eq!(
        before, after,
        "(quotes, quote_lines, benefit_reservations, Σ consumed_thb) unchanged (O-3, E-4); answer: {}",
        a.text
    );
    db.finish().await;
}

// ── Row 8 ───────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn be4_row8_another_organisations_customer_is_estimated_customer_less() {
    let db = TestDb::start().await;
    let gated = branch_with_usd(&db, "be4r8g").await;
    let open = branch_with_usd(&db, "be4r8o").await;

    // A customer of ORG_2 who would pass ORG_1's segment rule if the estimate used it: a member of the
    // version's snapshot with MARKETING consent granted.
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{ORG_2}', '{}', NULL, 'Asia/Bangkok', now())",
        unique("be4org2")
    ))
    .await;
    let foreign = unique("be4-cust-org2");
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('{foreign}', '{ORG_2}'::uuid, '{foreign}', 'Foreign Customer', 'LIVE', now())"
    ))
    .await;
    let snapshot = snapshot_of(&db, &foreign).await;
    consent(&db, ORG_1, &foreign).await;
    consent(&db, ORG_2, &foreign).await;

    // A rule needing a customer condition (segment, D-13) at one branch; an open rule at the other.
    campaign(&db, &gated, json!({ "segment": true }), Some(snapshot)).await;
    campaign(&db, &open, json!({}), None).await;

    let who = format!("org2:{foreign}");
    let a = preview_ok(&db, &who, &gated, "100").await;
    assert_eq!(
        a.body["benefit"]["state"], "NONE",
        "another organisation's customer is evaluated customer-less, so the segment rule does not apply (E-5): {}",
        a.text
    );
    assert_eq!(
        a.body["benefit"]["reason"], "notEligible",
        "a rule exists but none applies to this caller (E-7): {}",
        a.text
    );
    assert_eq!(a.body["benefit"]["basis"], "INDICATIVE", "{}", a.text);
    assert_eq!(a.body["benefit"]["channel"], "APP", "{}", a.text);
    assert!(
        a.body["benefit"].get("items").is_none(),
        "a NONE benefit carries no items: {}",
        a.text
    );

    let a = preview_ok(&db, &who, &open, "100").await;
    let b = assert_estimated(&a, 1);
    assert_ne!(
        s(b, "finalRate", &a.text),
        s(&a.body, "rate", &a.text),
        "the open rule still applies to a customer-less estimate: {}",
        a.text
    );
    db.finish().await;
}

// ── Row 10 ──────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn be4_row10_a_stale_line_its_policy_refuses_refuses_the_preview_as_today() {
    let db = TestDb::start().await;
    let branch = branch_id("be4r10r");
    db.branch(&branch).await;
    let stop = db.policy(&branch, Some("STOP"), None).await;
    db.publish(
        &branch,
        &[Line {
            stale_under: Some(stop),
            ..USD
        }],
    )
    .await;
    campaign(&db, &branch, json!({}), None).await;

    let a = preview(&db, "", &branch, "100").await;
    assert_eq!(
        a.status,
        StatusCode::CONFLICT,
        "a refused stale line refuses the preview: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"], "rateUnavailable",
        "refused as today (E-7): {}",
        a.text
    );
    assert!(
        a.body.get("benefit").is_none(),
        "a refused preview carries no benefit: {}",
        a.text
    );
    db.finish().await;
}

#[actix_web::test]
async fn be4_row10_a_stale_line_its_policy_admits_is_estimated_and_marked_stale() {
    let db = TestDb::start().await;
    let branch = branch_id("be4r10a");
    db.branch(&branch).await;
    let open = db.policy(&branch, None, None).await;
    db.publish(
        &branch,
        &[Line {
            stale_under: Some(open),
            ..USD
        }],
    )
    .await;
    campaign(&db, &branch, json!({}), None).await;

    let a = preview_ok(&db, "", &branch, "100").await;
    assert_eq!(
        a.body["stale"], true,
        "the admitted stale line is marked stale: {}",
        a.text
    );
    let b = assert_estimated(&a, 1);
    assert_ne!(
        s(b, "finalRate", &a.text),
        s(&a.body, "rate", &a.text),
        "an admitted stale line is still estimated: {}",
        a.text
    );
    db.finish().await;
}
