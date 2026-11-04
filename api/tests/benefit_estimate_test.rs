// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE under the xchange BE lane controller claude/bc8fd29a). Do not edit, delete or weaken.

//! Up-front tests for BE-3, `BenefitEstimator::estimate_in` (design `marketing-campaigns.md` §6.4, decisions
//! E-1…E-10, class diagram §6.4.4, behaviour table §6.4.5). Owner decisions (2026-10-27): OD-1 B (the estimate
//! keeps only rules that admit APP **and** DESK, `also_admitted_on = Some(Desk)`), OD-2 A (no coupon code), OD-3 A
//! (an anonymous caller gets a customer-less estimate).
//!
//! Rows of §6.4.5 covered here, one test each:
//! - 3: a DESK-only rule ⇒ `None(NotEligible)` (judged on APP, E-5);
//! - 4: an APP-only rule ⇒ `None(NotEligible)` (OD-1 B);
//! - 5: no ACTIVE rule for the organisation ⇒ `None(NoActiveRule)`;
//! - 6: a `requires_coupon` rule only ⇒ `None`; the coupon row is neither changed nor locked, no reservation;
//! - 7: a segment rule; a customer in the snapshot with MARKETING consent ⇒ `Estimated`; after withdrawing ⇒
//!   `None(NotEligible)` (D-13);
//! - 9: a version whose `consumed_thb ≥ budget_thb` is not estimated (the quote's budget filter);
//! - 11: an APP+DESK rule: the estimate's item `amount_thb`, `rate_after` and `final_rate` are byte-equal to the
//!   DESK quote's applied entry and line (D-7);
//! - 12: two applied rules from versions ending T1 < T2 ⇒ each item's `ends_at` is its version's, `valid_until = T1`;
//! - 13: an applied item's `label` is the campaign's `name`;
//! - 14: a `FEE_WAIVER` rule is evaluated on fee 0 ⇒ `amount_thb = "0.00"` (E-8).
//!
//! Every estimate runs inside a transaction the test rolls back (E-4). Each test owns a schema (`TestDb`) and a
//! branch of its own. Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_app::application::planner::planner_preview::{PlannerEstimate, PlannerPreview};
use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan};
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Side};
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::benefit_estimate::{
    BenefitEstimator, EstimateOutcome, NoBenefitReason,
};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

const QUANTITY: i64 = 100;

// ── Fixtures ────────────────────────────────────────────────────────────────

struct World {
    db: TestDb,
    branch: String,
    user: Uuid,
    now: DateTime<Utc>,
}

impl World {
    /// A schema of its own, `branch` publishing USD 1 at 32.00 / 33.00, and one user to author campaigns.
    async fn start(branch: &str) -> Self {
        let db = TestDb::start().await;
        db.branch(branch).await;
        xc_testdb::seed_branch_currencies(&db.pool, branch, &["USD"])
            .await
            .expect("seed branch currencies");
        db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
            .await;
        let user = Uuid::new_v4();
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{user}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
            unique("be3-user")
        ))
        .await;
        Self {
            db,
            branch: branch.to_owned(),
            user,
            now: whole_second_now(),
        }
    }

    /// The customer sells 100 USD: the shop BUYS at 32.00 (`CustomerSide::to_shop`).
    fn plan(&self) -> DraftPlan {
        DraftPlan {
            branch_id: Some(BranchId::new(&self.branch)),
            currency: CurrencyCode::parse("USD").expect("USD"),
            customer_side: CustomerSide::CustomerSell,
            amount: DraftAmount::Quantity(ForeignAmount::from_decimal(Decimal::from(QUANTITY))),
        }
    }
}

/// One campaign version to seed; `active` is an APPROVED running version on the world's branch.
struct VersionSpec<'a> {
    name: &'a str,
    status: &'a str,
    stack_group: &'a str,
    priority: i32,
    ends_at: DateTime<Utc>,
    budget_thb: &'a str,
    consumed_thb: &'a str,
    segment_snapshot: Option<Uuid>,
}

impl<'a> VersionSpec<'a> {
    fn active(w: &World, name: &'a str) -> Self {
        Self {
            name,
            status: "APPROVED",
            stack_group: "default",
            priority: 1,
            ends_at: w.now + Duration::days(30),
            budget_thb: "5000.00",
            consumed_thb: "0",
            segment_snapshot: None,
        }
    }
}

/// A campaign named `spec.name` with one version on the world's branch and its budget row; the version id.
async fn seed_version(w: &World, spec: VersionSpec<'_>) -> Uuid {
    let (campaign, version) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign)
    .bind(ORG_1)
    .bind(spec.name)
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert campaign");
    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, segment_snapshot_id, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, $4, $5, $6, $7::numeric, $8, $9, $10, 'hash-be3', now(), $11, now())",
    )
    .bind(version)
    .bind(ORG_1)
    .bind(campaign)
    .bind(spec.status)
    .bind(w.now - Duration::days(1))
    .bind(spec.ends_at)
    .bind(spec.budget_thb)
    .bind(spec.stack_group)
    .bind(spec.priority)
    .bind(spec.segment_snapshot)
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert campaign version");
    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version)
        .bind(campaign)
        .execute(&w.db.pool)
        .await
        .expect("set current version");
    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version)
        .bind(&w.branch)
        .execute(&w.db.pool)
        .await
        .expect("insert campaign branch");
    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, $2::numeric, 0)",
    )
    .bind(version)
    .bind(spec.consumed_thb)
    .execute(&w.db.pool)
    .await
    .expect("insert campaign budget");
    version
}

/// One rule to seed; `rate` is a PRICE `RATE_IMPROVEMENT` of +0.50 open to everyone.
struct RuleSpec {
    benefit_type: &'static str,
    benefit_class: &'static str,
    eligibility: Value,
    calculation: Value,
    requires_coupon: bool,
}

impl RuleSpec {
    fn rate(eligibility: Value) -> Self {
        Self {
            benefit_type: "RATE_IMPROVEMENT",
            benefit_class: "PRICE",
            eligibility,
            calculation: json!({ "delta": "0.50" }),
            requires_coupon: false,
        }
    }
}

async fn seed_rule(w: &World, version: Uuid, spec: RuleSpec) -> Uuid {
    let rule = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, \
             caps, requires_coupon, rule_no) \
         VALUES ($1, $2, $3, $4, $5, $6, '{}'::jsonb, $7, \
             (SELECT coalesce(max(rule_no), 0) + 1 FROM benefit_rules WHERE campaign_version_id = $2))",
    )
    .bind(rule)
    .bind(version)
    .bind(spec.benefit_type)
    .bind(spec.benefit_class)
    .bind(spec.eligibility)
    .bind(spec.calculation)
    .bind(spec.requires_coupon)
    .execute(&w.db.pool)
    .await
    .expect("insert benefit rule");
    rule
}

/// An active campaign named `name` with one rule of `spec`; (version id, rule id).
async fn campaign(w: &World, name: &str, spec: RuleSpec) -> (Uuid, Uuid) {
    let version = seed_version(w, VersionSpec::active(w, name)).await;
    let rule = seed_rule(w, version, spec).await;
    (version, rule)
}

async fn seed_consent(w: &World, customer: &str, granted: bool, effective_at: DateTime<Utc>) {
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, \
             effective_at, evidence_ref) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', $4, $5, 'test-be3')",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(customer)
    .bind(granted)
    .bind(effective_at)
    .execute(&w.db.pool)
    .await
    .expect("insert consent");
}

/// A segment snapshot whose only member is `customer`; its id.
async fn seed_snapshot(w: &World, customer: &str) -> Uuid {
    let (definition, snapshot) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, '{}'::jsonb, $4, now())",
    )
    .bind(definition)
    .bind(ORG_1)
    .bind(format!("seg-{definition}"))
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert segment definition");
    sqlx::query(
        "INSERT INTO segment_snapshots (id, organization_id, definition_id, as_of, member_count, taken_by) \
         VALUES ($1, $2::uuid, $3, now(), 1, $4)",
    )
    .bind(snapshot)
    .bind(ORG_1)
    .bind(definition)
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert segment snapshot");
    sqlx::query(
        "INSERT INTO segment_snapshot_members (snapshot_id, customer_id, metrics) VALUES ($1, $2, '{}'::jsonb)",
    )
    .bind(snapshot)
    .bind(customer)
    .execute(&w.db.pool)
    .await
    .expect("insert snapshot member");
    snapshot
}

/// An ISSUED coupon `code` for `rule`, valid for a day; its id.
async fn seed_coupon(w: &World, rule: Uuid, code: &str) -> Uuid {
    common::install_test_pii_keys();
    let keys = xc_customer::get_pii_keys().expect("test PII keys installed");
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, \
             issued_at) VALUES ($1, $2::uuid, $3, $4, 'ISSUED', NULL, $5, $6, now())",
    )
    .bind(id)
    .bind(ORG_1)
    .bind(rule)
    .bind(&CouponCodeHasher::new(&keys).hash(code)[..])
    .bind(w.now + Duration::days(1))
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert coupon");
    id
}

fn org() -> Uuid {
    Uuid::from_str(ORG_1).expect("ORG_1 is a uuid")
}

/// The plan's published estimate, read in `tx` (the line the benefit estimate prices).
async fn planner(
    w: &World,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: &DraftPlan,
) -> PlannerEstimate {
    PlannerPreview::new(w.db.pool.clone())
        .estimate(tx, plan, w.now)
        .await
        .unwrap_or_else(|e| panic!("planner estimate of {plan:?}: {e:?}"))
}

/// `BenefitEstimator::estimate_in` for `customer` in a transaction that is rolled back.
async fn estimate(w: &World, customer: Option<&str>) -> EstimateOutcome {
    let mut tx = w.db.pool.begin().await.expect("begin");
    let outcome = estimate_in_tx(w, &mut tx, customer).await;
    tx.rollback().await.expect("rollback");
    outcome
}

async fn estimate_in_tx(
    w: &World,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    customer: Option<&str>,
) -> EstimateOutcome {
    let plan = w.plan();
    let published = planner(w, tx, &plan).await;
    BenefitEstimator::new(Arc::new(PgCampaignRepository::new()))
        .estimate_in(
            tx,
            org(),
            &BranchId::new(&w.branch),
            customer,
            &published,
            &plan,
            w.now,
        )
        .await
        .unwrap_or_else(|e| panic!("estimate_in for {customer:?}: {e}"))
}

fn estimated(
    outcome: EstimateOutcome,
) -> xchange::application::workflows::exchange::benefit_estimate::BenefitEstimate {
    match outcome {
        EstimateOutcome::Estimated(e) => e,
        other => panic!("expected Estimated, got {other:?}"),
    }
}

fn teller() -> SaveActor {
    SaveActor {
        id: "teller-be3".to_owned(),
        username: "user-teller-be3".to_owned(),
        employee_id: Some("teller-be3".to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

/// The same plan as a one-line DESK quote (shop BUYS 100 USD), committed.
async fn desk_quote(w: &World) -> Quote {
    let req = QuoteRequest {
        branch_id: BranchId::new(&w.branch),
        customer_id: None,
        channel: QuoteChannel::Desk,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse("USD").expect("USD"),
            shop_side: Side::Buy,
            quantity: ForeignAmount::from_decimal(Decimal::from(QUANTITY)),
            fee_thb: None,
            denomination_group: GroupCode::all(),
        }],
        payment_method: None,
    };
    let mut tx = w.db.pool.begin().await.expect("begin");
    let q = QuoteService::new(w.db.pool.clone())
        .quote_with_coupon_in(&mut tx, req, &teller(), w.now, None)
        .await
        .unwrap_or_else(|e| panic!("DESK quote: {e}"));
    tx.commit().await.expect("commit");
    q
}

// ── Row 3 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row3_desk_only_rule_is_none_not_eligible() {
    let w = World::start("B-BE3-R3").await;
    campaign(
        &w,
        "be3-row3-desk-only",
        RuleSpec::rate(json!({ "currencies": ["USD"], "channels": ["DESK"] })),
    )
    .await;

    let outcome = estimate(&w, None).await;

    assert_eq!(
        outcome,
        EstimateOutcome::None(NoBenefitReason::NotEligible),
        "a DESK-only rule is judged on APP and does not apply (E-5); observed {outcome:?}"
    );
    w.db.finish().await;
}

// ── Row 4 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row4_app_only_rule_is_none_not_eligible_under_od1_b() {
    let w = World::start("B-BE3-R4").await;
    campaign(
        &w,
        "be3-row4-app-only",
        RuleSpec::rate(json!({ "currencies": ["USD"], "channels": ["APP"] })),
    )
    .await;

    let outcome = estimate(&w, None).await;

    assert_eq!(
        outcome,
        EstimateOutcome::None(NoBenefitReason::NotEligible),
        "OD-1 B: an APP-only rule cannot be given at the counter, so it is never estimated; observed {outcome:?}"
    );
    w.db.finish().await;
}

// ── Row 5 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row5_no_active_rule_is_none_no_active_rule() {
    let w = World::start("B-BE3-R5").await;
    // A PAUSED version is not ACTIVE: the organisation has no ACTIVE rule.
    let paused = seed_version(
        &w,
        VersionSpec {
            status: "PAUSED",
            ..VersionSpec::active(&w, "be3-row5-paused")
        },
    )
    .await;
    seed_rule(&w, paused, RuleSpec::rate(json!({ "currencies": ["USD"] }))).await;

    let outcome = estimate(&w, None).await;

    assert_eq!(
        outcome,
        EstimateOutcome::None(NoBenefitReason::NoActiveRule),
        "no ACTIVE rule ⇒ noActiveRule (E-7); observed {outcome:?}"
    );
    w.db.finish().await;
}

// ── Row 6 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row6_coupon_only_rule_is_none_and_no_coupon_is_locked() {
    let w = World::start("B-BE3-R6").await;
    let (_, rule) = campaign(
        &w,
        "be3-row6-coupon",
        RuleSpec {
            requires_coupon: true,
            ..RuleSpec::rate(json!({ "currencies": ["USD"], "requiresCoupon": true }))
        },
    )
    .await;
    let code = unique("BE3-CPN").to_uppercase();
    let coupon = seed_coupon(&w, rule, &code).await;

    let mut tx = w.db.pool.begin().await.expect("begin");
    let outcome = estimate_in_tx(&w, &mut tx, None).await;

    // While the estimate's transaction is still open, the coupon row must not be locked by it.
    let mut probe = w.db.pool.begin().await.expect("begin probe");
    let lock = sqlx::query("SELECT id FROM coupons WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(coupon)
        .fetch_optional(&mut *probe)
        .await;
    probe.rollback().await.expect("rollback probe");
    let reservations_in_tx: i64 = sqlx::query_scalar("SELECT count(*) FROM benefit_reservations")
        .fetch_one(&mut *tx)
        .await
        .expect("count reservations in the estimate's transaction");
    tx.rollback().await.expect("rollback");

    assert!(
        matches!(outcome, EstimateOutcome::None(_)),
        "OD-2 A: no coupon code is evaluated, so a requires_coupon rule never appears; observed {outcome:?}"
    );
    assert!(
        matches!(lock, Ok(Some(_))),
        "the estimate holds no lock on the coupon row; FOR UPDATE NOWAIT observed {lock:?}"
    );
    assert_eq!(
        reservations_in_tx, 0,
        "the estimate writes no benefit_reservations row; observed {reservations_in_tx}"
    );
    let status =
        w.db.text(&format!("SELECT status FROM coupons WHERE id = '{coupon}'"))
            .await;
    assert_eq!(
        status.as_deref(),
        Some("ISSUED"),
        "the coupon stays ISSUED; observed {status:?}"
    );
    let reservations =
        w.db.count("SELECT count(*) FROM benefit_reservations")
            .await;
    assert_eq!(
        reservations, 0,
        "no benefit_reservations row after the estimate; observed {reservations}"
    );
    w.db.finish().await;
}

// ── Row 7 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row7_segment_rule_needs_marketing_consent() {
    let w = World::start("B-BE3-R7").await;
    let customer = unique("be3-cust");
    w.db.customer(&customer, &w.branch).await;
    let snapshot = seed_snapshot(&w, &customer).await;
    seed_consent(&w, &customer, true, w.now - Duration::hours(2)).await;
    let version = seed_version(
        &w,
        VersionSpec {
            segment_snapshot: Some(snapshot),
            ..VersionSpec::active(&w, "be3-row7-segment")
        },
    )
    .await;
    let rule = seed_rule(
        &w,
        version,
        RuleSpec::rate(json!({ "currencies": ["USD"], "segment": true })),
    )
    .await;

    let granted = estimated(estimate(&w, Some(&customer)).await);
    assert_eq!(
        granted.items.iter().map(|i| i.rule_id).collect::<Vec<_>>(),
        vec![rule],
        "a segment member with MARKETING consent gets the segment rule; observed {:?}",
        granted.items
    );

    seed_consent(&w, &customer, false, w.now - Duration::hours(1)).await;
    let withdrawn = estimate(&w, Some(&customer)).await;
    assert_eq!(
        withdrawn,
        EstimateOutcome::None(NoBenefitReason::NotEligible),
        "after withdrawing MARKETING consent the segment rule no longer applies (D-13); observed {withdrawn:?}"
    );
    w.db.finish().await;
}

// ── Row 9 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row9_exhausted_budget_version_is_not_estimated() {
    let w = World::start("B-BE3-R9").await;
    // The exhausted version would win (higher priority, same stack group) if it were estimated.
    let exhausted = seed_version(
        &w,
        VersionSpec {
            priority: 5,
            budget_thb: "100.00",
            consumed_thb: "100.00",
            ..VersionSpec::active(&w, "be3-row9-exhausted")
        },
    )
    .await;
    let exhausted_rule = seed_rule(
        &w,
        exhausted,
        RuleSpec {
            calculation: json!({ "delta": "0.80" }),
            ..RuleSpec::rate(json!({ "currencies": ["USD"] }))
        },
    )
    .await;

    let only_exhausted = estimate(&w, None).await;
    assert!(
        matches!(only_exhausted, EstimateOutcome::None(_)),
        "a version whose consumed_thb ≥ budget_thb is not estimated; observed {only_exhausted:?}"
    );

    let (_, open_rule) = campaign(
        &w,
        "be3-row9-open",
        RuleSpec::rate(json!({ "currencies": ["USD"] })),
    )
    .await;
    let est = estimated(estimate(&w, None).await);
    let rules: Vec<Uuid> = est.items.iter().map(|i| i.rule_id).collect();
    assert_eq!(
        rules,
        vec![open_rule],
        "only the version with budget left is estimated (exhausted rule {exhausted_rule}); observed {rules:?}"
    );
    w.db.finish().await;
}

// ── Row 11 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row11_estimate_is_byte_equal_to_the_desk_quote() {
    let w = World::start("B-BE3-R11").await;
    let (_, rule) = campaign(
        &w,
        "be3-row11-app-desk",
        RuleSpec::rate(json!({ "currencies": ["USD"], "channels": ["APP", "DESK"] })),
    )
    .await;

    let est = estimated(estimate(&w, None).await);
    let q = desk_quote(&w).await;

    let applied = q
        .benefit_snapshot
        .get("applied")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        applied.len(),
        1,
        "the DESK quote applies the rule once; observed {applied:?}"
    );
    assert_eq!(
        est.items.len(),
        1,
        "the estimate has one item; observed {:?}",
        est.items
    );
    let item = &est.items[0];
    assert_eq!(
        item.rule_id, rule,
        "the item is the rule; observed {item:?}"
    );
    assert_eq!(
        Some(item.amount_thb.as_str()),
        applied[0]["amountThb"].as_str(),
        "item amount_thb {:?} vs the quote's applied amountThb {}",
        item.amount_thb,
        applied[0]["amountThb"]
    );
    assert_eq!(
        Some(item.rate_after.as_str()),
        applied[0]["rateAfter"].as_str(),
        "item rate_after {:?} vs the quote's applied rateAfter {}",
        item.rate_after,
        applied[0]["rateAfter"]
    );
    assert_eq!(
        est.final_rate.to_string(),
        q.lines[0].final_rate.to_string(),
        "estimate final_rate {} vs the quote line's final_rate {}",
        est.final_rate,
        q.lines[0].final_rate
    );
    assert_eq!(
        est.thb_after.to_string(),
        q.lines[0].thb_amount.to_string(),
        "estimate thb_after {} vs the quote line's thb_amount {}",
        est.thb_after,
        q.lines[0].thb_amount
    );
    assert_eq!(
        est.publication_id, q.publication_id,
        "the estimate priced the quote's publication; observed {} vs {}",
        est.publication_id, q.publication_id
    );
    w.db.finish().await;
}

// ── Row 12 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row12_items_carry_their_version_end_and_valid_until_is_the_earliest() {
    let w = World::start("B-BE3-R12").await;
    let t1 = w.now + Duration::days(10);
    let t2 = w.now + Duration::days(20);
    // Two stack groups: one PRICE and one REWARD winner, so both apply to the one line.
    let late = seed_version(
        &w,
        VersionSpec {
            stack_group: "group-price",
            ends_at: t2,
            ..VersionSpec::active(&w, "be3-row12-ends-t2")
        },
    )
    .await;
    let late_rule = seed_rule(&w, late, RuleSpec::rate(json!({ "currencies": ["USD"] }))).await;
    let early = seed_version(
        &w,
        VersionSpec {
            stack_group: "group-reward",
            ends_at: t1,
            ..VersionSpec::active(&w, "be3-row12-ends-t1")
        },
    )
    .await;
    let early_rule = seed_rule(
        &w,
        early,
        RuleSpec {
            benefit_class: "REWARD",
            calculation: json!({ "delta": "0.10" }),
            ..RuleSpec::rate(json!({ "currencies": ["USD"] }))
        },
    )
    .await;

    let est = estimated(estimate(&w, None).await);

    assert_eq!(
        est.items.len(),
        2,
        "both rules apply; observed {:?}",
        est.items
    );
    let ends_of = |rule: Uuid| {
        est.items
            .iter()
            .find(|i| i.rule_id == rule)
            .map(|i| i.ends_at)
            .unwrap_or_else(|| panic!("rule {rule} is an item; observed {:?}", est.items))
    };
    assert_eq!(
        ends_of(late_rule),
        t2,
        "the T2 rule's ends_at is its version's; observed {}",
        ends_of(late_rule)
    );
    assert_eq!(
        ends_of(early_rule),
        t1,
        "the T1 rule's ends_at is its version's; observed {}",
        ends_of(early_rule)
    );
    assert_eq!(
        est.valid_until, t1,
        "valid_until is the earliest ends_at (E-6); observed {}",
        est.valid_until
    );
    w.db.finish().await;
}

// ── Row 13 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row13_item_label_is_the_campaign_name() {
    let w = World::start("B-BE3-R13").await;
    let name = unique("be3-row13-Songkran-bonus");
    let (_, rule) = campaign(&w, &name, RuleSpec::rate(json!({ "currencies": ["USD"] }))).await;

    let est = estimated(estimate(&w, None).await);

    assert_eq!(est.items.len(), 1, "one item; observed {:?}", est.items);
    assert_eq!(
        est.items[0].rule_id, rule,
        "the item is the rule; observed {:?}",
        est.items[0]
    );
    assert_eq!(
        est.items[0].label, name,
        "label is the campaign's name; observed {:?}",
        est.items[0].label
    );
    w.db.finish().await;
}

// ── Row 14 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn be3_row14_fee_waiver_is_evaluated_on_fee_zero() {
    let w = World::start("B-BE3-R14").await;
    let (_, rule) = campaign(
        &w,
        "be3-row14-fee-waiver",
        RuleSpec {
            benefit_type: "FEE_WAIVER",
            calculation: json!({ "percent": "1" }),
            ..RuleSpec::rate(json!({ "currencies": ["USD"] }))
        },
    )
    .await;

    let est = estimated(estimate(&w, None).await);

    assert_eq!(
        est.items.len(),
        1,
        "the FEE_WAIVER applies; observed {:?}",
        est.items
    );
    let item = &est.items[0];
    assert_eq!(
        item.rule_id, rule,
        "the item is the rule; observed {item:?}"
    );
    assert_eq!(
        item.benefit_type, "FEE_WAIVER",
        "observed benefit_type {:?}",
        item.benefit_type
    );
    assert_eq!(
        item.amount_thb, "0.00",
        "a plan has no fee, so the waiver is worth 0.00 (E-8); observed {:?}",
        item.amount_thb
    );
    w.db.finish().await;
}
