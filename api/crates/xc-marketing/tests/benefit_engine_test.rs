//! Up-front tests for MK-2: benefit domain + engine (design marketing-campaigns.md §9, §9.1, #1392).
//!
//! Behaviours locked:
//! - each calculator moves the rate toward the customer on BUY and on SELL (§9.1);
//! - PRICE changes final_rate / fee_thb (§9.1);
//! - the margin guard caps PRICE benefits at max_spread_share of the spread and does nothing when it is NULL (§9.1, D-11);
//! - the floor caps a crossing rule and marks it floorCapped: true (§9.1, D-11);
//! - within a stack group the highest priority wins (§9.1);
//! - across groups at most one PRICE and one REWARD per line, never two PRICE (§9.1, D-10, M1);
//! - two PRICE rules in different stack groups -> only the higher priority applies (§9 row 1);
//! - equal priority in one group -> the older version id wins (§9 row 2, D-10);
//! - a rule that would cross the floor -> capped, floorCapped: true (§9 row 3);
//! - max_spread_share NULL vs 0 vs 1 (§9 row 4);
//! - anonymous quote with a tier rule -> skipped with tierMismatch (§9 row 5, §4.3);
//! - known customer with unlisted tier -> skipped with tierMismatch (§4.3);
//! - segment rule for a customer whose ConsentGate answers false -> skipped with notInSegment (§9 row 6, D-13, ruling);
//! - segment rule for a non-member with consent true -> skipped with notInSegment (D-13);
//! - positive control: member + consent true -> applied, segmentSnapshotIds set (D-13);
//! - branches miss -> skipped with branchMismatch (§4.3);
//! - referred miss -> skipped with notReferred (D-12, §4.3);
//! - every unmet condition yields its skip reason (§9.1, §4.3);
//! - same inputs => same output (pure engine) (§9.1, D-7);
//! - PgCampaignRepository::active_for DB-backed query returns APPROVED/active rules for branch (ruling 5).

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::{BenefitCalculator, BenefitEffect};
use xc_marketing::domain::benefit::eligibility::Eligibility;
use xc_marketing::domain::benefit::kinds::fee_waiver::FeeWaiver;
use xc_marketing::domain::benefit::kinds::rate_improvement::RateImprovement;
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};
use xc_marketing::domain::engine::active_rules::{ActiveRule, ActiveRules};
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::engine::engine::{BenefitEngine, EvalContext, QuoteLineInput};
use xc_marketing::domain::engine::referral_source::ReferralSource;
use xc_marketing::domain::engine::tier_source::TierSource;
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;

// ---------------------------------------------------------------------------
// DB test harness
// ---------------------------------------------------------------------------

static TEST_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
    });

static TEST_POOL: std::sync::OnceLock<sqlx::PgPool> = std::sync::OnceLock::new();

async fn test_pool() -> &'static sqlx::PgPool {
    TEST_POOL.get_or_init(|| {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The tests need a real PostgreSQL and do not skip.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        assert!(
            url.ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in _test"
        );
        std::thread::spawn(move || {
            TEST_RUNTIME.block_on(async {
                sqlx::postgres::PgPoolOptions::new()
                    .min_connections(12)
                    .max_connections(24)
                    .acquire_timeout(std::time::Duration::from_secs(30))
                    .connect(&url)
                    .await
                    .expect("connect to test database")
            })
        })
        .join()
        .expect("join pool init thread")
    })
}

async fn test_tx() -> Transaction<'static, Postgres> {
    test_pool().await.begin().await.expect("begin transaction")
}

// ---------------------------------------------------------------------------
// Test doubles / mock sources
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockActiveRules {
    rules: Vec<ActiveRule>,
}

impl ActiveRules for MockActiveRules {
    fn active_for<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _organization_id: Uuid,
        _branch: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ActiveRule>>> {
        let rules = self.rules.clone();
        Box::pin(async move { Ok(rules) })
    }
}

#[derive(Clone, Default)]
struct MockTierSource {
    tiers: HashMap<String, String>,
}

impl TierSource for MockTierSource {
    fn tier_of(&self, _org: Uuid, customer: &str) -> Option<String> {
        self.tiers.get(customer).cloned()
    }
}

#[derive(Clone, Default)]
struct MockConsentGate {
    allowed: HashMap<String, bool>,
}

impl ConsentGate for MockConsentGate {
    fn marketing_allowed(&self, customer: &str) -> bool {
        self.allowed.get(customer).copied().unwrap_or(false)
    }
}

#[derive(Clone, Default)]
struct MockReferralSource {
    referred: HashMap<String, bool>,
}

impl ReferralSource for MockReferralSource {
    fn referred(&self, customer: &str, _at: DateTime<Utc>) -> bool {
        self.referred.get(customer).copied().unwrap_or(false)
    }
}

// Dummy REWARD calculator for stacking tests
struct DummyRewardCalculator;
impl BenefitCalculator for DummyRewardCalculator {
    fn kind(&self) -> &str {
        "GIFT_VOUCHER"
    }
    fn class(&self) -> BenefitClass {
        BenefitClass::Reward
    }
    fn apply(&self, _line: &QuoteLineInput, _params: &serde_json::Value) -> BenefitEffect {
        BenefitEffect {
            new_final_rate: None,
            new_fee_thb: None,
            amount_thb: Money::from_minor(10000).unwrap(), // 100.00 THB
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn org_id() -> Uuid {
    Uuid::from_str("00000000-0000-0000-0000-000000000001").unwrap()
}

fn test_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap()
}

fn make_line(
    side: Side,
    base_rate_str: &str,
    buy_rate_str: &str,
    sell_rate_str: &str,
    qty_str: &str,
    fee_str: &str,
) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: side,
        quantity: ForeignAmount::from_decimal(Decimal::from_str(qty_str).unwrap()),
        unit_size: UnitSize::parse("1").unwrap(),
        base_rate: Rate::parse(base_rate_str).unwrap(),
        fee_thb: Money::parse(fee_str).unwrap(),
        published_buy_rate: Rate::parse(buy_rate_str).unwrap(),
        published_sell_rate: Rate::parse(sell_rate_str).unwrap(),
    }
}

fn make_context(
    customer_id: Option<&str>,
    coupon_code: Option<&str>,
    max_spread_share: Option<Decimal>,
    segment_memberships: Vec<Uuid>,
) -> EvalContext {
    EvalContext {
        organization_id: org_id(),
        branch: "MAIN".to_string(),
        channel: "DESK".to_string(),
        customer_id: customer_id.map(ToString::to_string),
        coupon_code: coupon_code.map(ToString::to_string),
        now: test_time(),
        max_spread_share,
        segment_memberships,
    }
}

#[derive(Debug, Clone)]
struct ActiveRuleSpec {
    pub rule_id: Uuid,
    pub version_id: Uuid,
    pub version_created_at: DateTime<Utc>,
    pub stack_group: &'static str,
    pub priority: i32,
    pub benefit_type: &'static str,
    pub benefit_class: BenefitClass,
    pub eligibility: Eligibility,
    pub calculation: serde_json::Value,
    pub segment_snapshot_id: Option<Uuid>,
}

impl Default for ActiveRuleSpec {
    fn default() -> Self {
        Self {
            rule_id: Uuid::new_v4(),
            version_id: Uuid::new_v4(),
            version_created_at: test_time(),
            stack_group: "default-group",
            priority: 10,
            benefit_type: "RATE_IMPROVEMENT",
            benefit_class: BenefitClass::Price,
            eligibility: Eligibility::default(),
            calculation: json!({ "delta": "0.10" }),
            segment_snapshot_id: None,
        }
    }
}

fn make_active_rule(spec: ActiveRuleSpec) -> ActiveRule {
    ActiveRule {
        rule: BenefitRule {
            id: spec.rule_id,
            campaign_version_id: spec.version_id,
            benefit_type: spec.benefit_type.to_string(),
            benefit_class: spec.benefit_class,
            eligibility: spec.eligibility,
            calculation: spec.calculation,
            caps: BenefitCaps::default(),
            requires_coupon: false,
            rule_no: 1,
        },
        stack_group: spec.stack_group.to_string(),
        priority: spec.priority,
        version_id: spec.version_id,
        version_created_at: spec.version_created_at,
        segment_snapshot_id: spec.segment_snapshot_id,
    }
}

// ---------------------------------------------------------------------------
// Tests covering §9 rows and §9.1 behaviours
// ---------------------------------------------------------------------------

/// §9 row 1, §9.1: Two PRICE rules in different stack groups -> only the higher priority applies (M1).
#[tokio::test]
async fn test_stacking_two_price_rules_different_groups_higher_priority_wins() {
    let rule_a_id = Uuid::new_v4();
    let rule_b_id = Uuid::new_v4();
    let version_a = Uuid::new_v4();
    let version_b = Uuid::new_v4();

    // Group A: priority 10
    let rule_a = make_active_rule(ActiveRuleSpec {
        rule_id: rule_a_id,
        version_id: version_a,
        stack_group: "group-alpha",
        priority: 10,
        calculation: json!({ "delta": "0.10" }),
        ..Default::default()
    });

    // Group B: priority 20 (higher)
    let rule_b = make_active_rule(ActiveRuleSpec {
        rule_id: rule_b_id,
        version_id: version_b,
        stack_group: "group-beta",
        priority: 20,
        calculation: json!({ "delta": "0.20" }),
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![rule_a, rule_b],
    };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    // Only the higher priority PRICE rule (Rule B, priority 20) applies
    assert_eq!(evaluation.snapshot.applied.len(), 1);
    assert_eq!(evaluation.snapshot.applied[0].rule_id, rule_b_id);
    assert_eq!(
        evaluation.lines[0].final_rate,
        Rate::parse("35.20").unwrap()
    );
}

/// §9 row 2, D-10: Equal priority in one group -> the older version id wins.
#[tokio::test]
async fn test_stacking_equal_priority_older_version_wins() {
    let rule_older_id = Uuid::new_v4();
    let rule_newer_id = Uuid::new_v4();
    let version_older = Uuid::from_str("00000000-0000-0000-0000-000000000001").unwrap();
    let version_newer = Uuid::from_str("00000000-0000-0000-0000-000000000002").unwrap();

    let time_older = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    let time_newer = Utc.with_ymd_and_hms(2026, 10, 5, 0, 0, 0).unwrap();

    // Equal priority 15 in the same stack group
    let rule_older = make_active_rule(ActiveRuleSpec {
        rule_id: rule_older_id,
        version_id: version_older,
        version_created_at: time_older,
        stack_group: "shared-group",
        priority: 15,
        calculation: json!({ "delta": "0.15" }),
        ..Default::default()
    });

    let rule_newer = make_active_rule(ActiveRuleSpec {
        rule_id: rule_newer_id,
        version_id: version_newer,
        version_created_at: time_newer,
        stack_group: "shared-group",
        priority: 15,
        calculation: json!({ "delta": "0.25" }),
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![rule_newer, rule_older],
    };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    // Older version wins the tie
    assert_eq!(evaluation.snapshot.applied.len(), 1);
    assert_eq!(evaluation.snapshot.applied[0].rule_id, rule_older_id);
    assert_eq!(evaluation.snapshot.applied[0].version_id, version_older);
    assert_eq!(
        evaluation.lines[0].final_rate,
        Rate::parse("35.15").unwrap()
    );
}

/// §9 row 3, §9.1, D-11: A rule that would cross the floor -> capped, floorCapped: true (BUY and SELL).
#[tokio::test]
async fn test_floor_capped_on_buy_and_sell() {
    let rule_buy_id = Uuid::new_v4();
    let rule_sell_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    // +1.50 improvement on buy: base 35.00 + 1.50 = 36.50 > publication sell rate 36.00 -> floor capped to 36.00
    let rule_buy = make_active_rule(ActiveRuleSpec {
        rule_id: rule_buy_id,
        version_id,
        stack_group: "floor-buy",
        priority: 10,
        eligibility: Eligibility {
            shop_side: Some(Side::Buy),
            ..Default::default()
        },
        calculation: json!({ "delta": "1.50" }),
        ..Default::default()
    });

    // +1.50 improvement toward customer on sell: base 36.00 - 1.50 = 34.50 < publication buy rate 35.00 -> floor capped to 35.00
    let rule_sell = make_active_rule(ActiveRuleSpec {
        rule_id: rule_sell_id,
        version_id,
        stack_group: "floor-sell",
        priority: 10,
        eligibility: Eligibility {
            shop_side: Some(Side::Sell),
            ..Default::default()
        },
        calculation: json!({ "delta": "1.50" }),
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![rule_buy, rule_sell],
    };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);

    // 1. BUY line
    let buy_lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];
    let mut tx_buy = test_tx().await;
    let eval_buy = engine
        .evaluate(&mut tx_buy, &ctx, &buy_lines)
        .await
        .unwrap();
    assert_eq!(eval_buy.snapshot.applied.len(), 1);
    assert_eq!(eval_buy.snapshot.applied[0].rule_id, rule_buy_id);
    assert!(eval_buy.snapshot.applied[0].floor_capped);
    assert_eq!(eval_buy.lines[0].final_rate, Rate::parse("36.00").unwrap());

    // 2. SELL line
    let sell_lines = vec![make_line(
        Side::Sell,
        "36.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];
    let mut tx_sell = test_tx().await;
    let eval_sell = engine
        .evaluate(&mut tx_sell, &ctx, &sell_lines)
        .await
        .unwrap();
    assert_eq!(eval_sell.snapshot.applied.len(), 1);
    assert_eq!(eval_sell.snapshot.applied[0].rule_id, rule_sell_id);
    assert!(eval_sell.snapshot.applied[0].floor_capped);
    assert_eq!(eval_sell.lines[0].final_rate, Rate::parse("35.00").unwrap());
}

/// §9 row 4, §9.1, D-11: max_spread_share NULL vs 0 vs 1 (margin guard).
/// Note: per Ruling 4, no guardCapped field is asserted in the snapshot.
#[tokio::test]
async fn test_margin_guard_null_vs_zero_vs_one() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    // Improvement of 0.80 on spread of 1.00 (sell 36.00 - buy 35.00 = 1.00 spread)
    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "guard-group",
        priority: 10,
        calculation: json!({ "delta": "0.80" }),
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    // Case A: max_spread_share is NULL -> only the floor applies, 0.80 applied in full
    let ctx_null = make_context(None, None, None, vec![]);
    let mut tx_null = test_tx().await;
    let eval_null = engine
        .evaluate(&mut tx_null, &ctx_null, &lines)
        .await
        .unwrap();
    assert_eq!(eval_null.snapshot.applied.len(), 1);
    assert_eq!(eval_null.lines[0].final_rate, Rate::parse("35.80").unwrap());
    assert!(!eval_null.snapshot.applied[0].floor_capped);

    // Case B: max_spread_share is 0 -> 0% give-away allowed, capped to base_rate (35.00)
    let ctx_zero = make_context(None, None, Some(Decimal::ZERO), vec![]);
    let mut tx_zero = test_tx().await;
    let eval_zero = engine
        .evaluate(&mut tx_zero, &ctx_zero, &lines)
        .await
        .unwrap();
    assert_eq!(eval_zero.snapshot.applied.len(), 1);
    assert_eq!(eval_zero.lines[0].final_rate, Rate::parse("35.00").unwrap());

    // Case C: max_spread_share is 1 -> 100% of spread allowed (up to 36.00), 0.80 allowed in full
    let ctx_one = make_context(None, None, Some(Decimal::ONE), vec![]);
    let mut tx_one = test_tx().await;
    let eval_one = engine
        .evaluate(&mut tx_one, &ctx_one, &lines)
        .await
        .unwrap();
    assert_eq!(eval_one.snapshot.applied.len(), 1);
    assert_eq!(eval_one.lines[0].final_rate, Rate::parse("35.80").unwrap());

    // Case D: max_spread_share is 0.40 -> 40% of spread (0.40) allowed; 0.80 capped to +0.40 -> 35.40
    let ctx_forty = make_context(None, None, Some(Decimal::from_str("0.40").unwrap()), vec![]);
    let mut tx_forty = test_tx().await;
    let eval_forty = engine
        .evaluate(&mut tx_forty, &ctx_forty, &lines)
        .await
        .unwrap();
    assert_eq!(eval_forty.snapshot.applied.len(), 1);
    assert_eq!(
        eval_forty.lines[0].final_rate,
        Rate::parse("35.40").unwrap()
    );
}

/// §9 row 5, §4.3: Anonymous quote with a tier rule -> skipped with tierMismatch.
#[tokio::test]
async fn test_anonymous_quote_tier_rule_skipped_tier_mismatch() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "tier-group",
        priority: 10,
        eligibility: Eligibility {
            tiers: Some(vec!["PLATINUM".to_string(), "GOLD".to_string()]),
            ..Default::default()
        },
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    // Anonymous quote: customer_id is None
    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert!(evaluation.snapshot.applied.is_empty());
    assert_eq!(evaluation.snapshot.skipped.len(), 1);
    assert_eq!(evaluation.snapshot.skipped[0].rule_id, rule_id);
    assert_eq!(evaluation.snapshot.skipped[0].reason, "tierMismatch");
    assert_eq!(
        evaluation.lines[0].final_rate,
        Rate::parse("35.00").unwrap()
    );
}

/// §4.3: A known customer whose tier is not in tiers -> skipped with tierMismatch.
#[tokio::test]
async fn test_known_customer_unlisted_tier_skipped_tier_mismatch() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "tier-group",
        priority: 10,
        eligibility: Eligibility {
            tiers: Some(vec!["GOLD".to_string(), "PLATINUM".to_string()]),
            ..Default::default()
        },
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let mut tiers = HashMap::new();
    tiers.insert("cust-silver".to_string(), "SILVER".to_string());

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource { tiers }),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(Some("cust-silver"), None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert!(evaluation.snapshot.applied.is_empty());
    assert_eq!(evaluation.snapshot.skipped.len(), 1);
    assert_eq!(evaluation.snapshot.skipped[0].rule_id, rule_id);
    assert_eq!(evaluation.snapshot.skipped[0].reason, "tierMismatch");
}

/// §9 row 6, D-13, controller ruling: Segment rule for a customer whose ConsentGate answers false -> skipped with notInSegment.
#[tokio::test]
async fn test_consent_gate_false_skipped_not_in_segment() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let segment_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "seg-group",
        priority: 10,
        eligibility: Eligibility {
            segment: true,
            ..Default::default()
        },
        segment_snapshot_id: Some(segment_id),
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    // Customer is in segment snapshot, but ConsentGate answers false
    let mut consent = MockConsentGate::default();
    consent.allowed.insert("cust-no-consent".to_string(), false);

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(consent),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(
        Some("cust-no-consent"),
        None,
        None,
        vec![segment_id], // member of segment
    );
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    // Per D-13 and controller ruling: unmet segment condition yields notInSegment
    assert!(evaluation.snapshot.applied.is_empty());
    assert_eq!(evaluation.snapshot.skipped.len(), 1);
    assert_eq!(evaluation.snapshot.skipped[0].rule_id, rule_id);
    assert_eq!(evaluation.snapshot.skipped[0].reason, "notInSegment");
}

/// D-13: A non-member with consent true -> skipped with notInSegment.
#[tokio::test]
async fn test_non_member_with_consent_skipped_not_in_segment() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let segment_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "seg-group",
        priority: 10,
        eligibility: Eligibility {
            segment: true,
            ..Default::default()
        },
        segment_snapshot_id: Some(segment_id),
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    // ConsentGate answers true, but customer is NOT in segment_memberships
    let mut consent = MockConsentGate::default();
    consent
        .allowed
        .insert("cust-consent-nonmember".to_string(), true);

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(consent),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(
        Some("cust-consent-nonmember"),
        None,
        None,
        vec![], // not in segment
    );
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert!(evaluation.snapshot.applied.is_empty());
    assert_eq!(evaluation.snapshot.skipped.len(), 1);
    assert_eq!(evaluation.snapshot.skipped[0].rule_id, rule_id);
    assert_eq!(evaluation.snapshot.skipped[0].reason, "notInSegment");
}

/// D-13 positive control: member + consent true -> the rule applies, and segmentSnapshotIds is set.
#[tokio::test]
async fn test_segment_rule_positive_control_member_with_consent_applies() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let segment_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "seg-group",
        priority: 10,
        eligibility: Eligibility {
            segment: true,
            ..Default::default()
        },
        calculation: json!({ "delta": "0.10" }),
        segment_snapshot_id: Some(segment_id),
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    // Member with granted consent
    let mut consent = MockConsentGate::default();
    consent.allowed.insert("cust-member".to_string(), true);

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(consent),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(
        Some("cust-member"),
        None,
        None,
        vec![segment_id], // member of segment
    );
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(evaluation.snapshot.applied.len(), 1);
    assert_eq!(evaluation.snapshot.applied[0].rule_id, rule_id);
    assert!(
        evaluation
            .snapshot
            .segment_snapshot_ids
            .contains(&segment_id)
    );
    assert_eq!(
        evaluation.lines[0].final_rate,
        Rate::parse("35.10").unwrap()
    );
}

/// §4.3: branches miss -> skipped with branchMismatch.
#[tokio::test]
async fn test_branch_mismatch_skipped() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "branch-group",
        priority: 10,
        eligibility: Eligibility {
            branches: Some(vec!["AIRPORT".to_string()]),
            ..Default::default()
        },
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    // Context is branch "MAIN"
    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert!(evaluation.snapshot.applied.is_empty());
    assert_eq!(evaluation.snapshot.skipped.len(), 1);
    assert_eq!(evaluation.snapshot.skipped[0].rule_id, rule_id);
    assert_eq!(evaluation.snapshot.skipped[0].reason, "branchMismatch");
}

/// D-12, §4.3: referred miss -> skipped with notReferred.
#[tokio::test]
async fn test_referred_miss_skipped_not_referred() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "ref-group",
        priority: 10,
        eligibility: Eligibility {
            referred: true,
            ..Default::default()
        },
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let mut referral = MockReferralSource::default();
    referral
        .referred
        .insert("cust-unreferred".to_string(), false);

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(referral),
    );

    let ctx = make_context(Some("cust-unreferred"), None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert!(evaluation.snapshot.applied.is_empty());
    assert_eq!(evaluation.snapshot.skipped.len(), 1);
    assert_eq!(evaluation.snapshot.skipped[0].rule_id, rule_id);
    assert_eq!(evaluation.snapshot.skipped[0].reason, "notReferred");
}

/// §9.1: Each calculator moves the rate toward the customer on BUY and on SELL.
#[test]
fn test_calculators_move_rate_toward_customer_buy_and_sell() {
    let rate_calc = RateImprovement;
    let fee_calc = FeeWaiver;

    // BUY: shop buys foreign currency from customer; higher rate benefits customer
    let buy_line = make_line(Side::Buy, "35.00", "35.00", "36.00", "100", "50.00");
    let buy_effect = rate_calc.apply(&buy_line, &json!({ "delta": "0.10" }));
    assert_eq!(
        buy_effect.new_final_rate,
        Some(Rate::parse("35.10").unwrap())
    );

    // SELL: shop sells foreign currency to customer; lower rate benefits customer
    let sell_line = make_line(Side::Sell, "36.00", "35.00", "36.00", "100", "50.00");
    let sell_effect = rate_calc.apply(&sell_line, &json!({ "delta": "0.10" }));
    assert_eq!(
        sell_effect.new_final_rate,
        Some(Rate::parse("35.90").unwrap())
    );

    // Fee waiver reduces or clears the fee
    let fee_effect = fee_calc.apply(&buy_line, &json!({ "percent": "1.00" }));
    assert_eq!(fee_effect.new_fee_thb, Some(Money::ZERO));
}

/// §9.1: PRICE changes final_rate / fee_thb.
#[tokio::test]
async fn test_price_changes_final_rate_and_fee_thb() {
    let rate_rule_id = Uuid::new_v4();
    let fee_rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rate_rule = make_active_rule(ActiveRuleSpec {
        rule_id: rate_rule_id,
        version_id,
        stack_group: "rate-group",
        priority: 10,
        eligibility: Eligibility {
            shop_side: Some(Side::Buy),
            ..Default::default()
        },
        calculation: json!({ "delta": "0.25" }),
        ..Default::default()
    });

    let fee_rule = make_active_rule(ActiveRuleSpec {
        rule_id: fee_rule_id,
        version_id,
        stack_group: "fee-group",
        priority: 10,
        benefit_type: "FEE_WAIVER",
        eligibility: Eligibility {
            shop_side: Some(Side::Sell),
            ..Default::default()
        },
        calculation: json!({ "percent": "1.00" }),
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![rate_rule, fee_rule],
    };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![
        make_line(Side::Buy, "35.00", "35.00", "36.00", "100", "50.00"),
        make_line(Side::Sell, "36.00", "35.00", "36.00", "100", "50.00"),
    ];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    // Line 0: rate improvement changes final_rate
    let line_0 = &evaluation.lines[0];
    assert_ne!(line_0.final_rate, line_0.base_rate);
    assert_eq!(line_0.final_rate, Rate::parse("35.25").unwrap());

    // Line 1: fee waiver changes fee_thb
    let line_1 = &evaluation.lines[1];
    assert_eq!(line_1.final_rate, line_1.base_rate);
    assert_ne!(line_1.fee_thb, line_1.fee_before);
    assert_eq!(line_1.fee_thb, Money::ZERO);
}

/// §9.1: Within a stack group the highest priority wins.
#[tokio::test]
async fn test_stacking_within_group_highest_priority_wins() {
    let rule_low_id = Uuid::new_v4();
    let rule_high_id = Uuid::new_v4();
    let rule_mid_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule_low = make_active_rule(ActiveRuleSpec {
        rule_id: rule_low_id,
        version_id,
        stack_group: "same-group",
        priority: 5,
        calculation: json!({ "delta": "0.05" }),
        ..Default::default()
    });

    let rule_high = make_active_rule(ActiveRuleSpec {
        rule_id: rule_high_id,
        version_id,
        stack_group: "same-group",
        priority: 50,
        calculation: json!({ "delta": "0.50" }),
        ..Default::default()
    });

    let rule_mid = make_active_rule(ActiveRuleSpec {
        rule_id: rule_mid_id,
        version_id,
        stack_group: "same-group",
        priority: 25,
        calculation: json!({ "delta": "0.25" }),
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![rule_low, rule_high, rule_mid],
    };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(evaluation.snapshot.applied.len(), 1);
    assert_eq!(evaluation.snapshot.applied[0].rule_id, rule_high_id);
    assert_eq!(
        evaluation.lines[0].final_rate,
        Rate::parse("35.50").unwrap()
    );
}

/// §9.1, D-10, M1: Across groups at most one PRICE and one REWARD per line, never two PRICE.
#[tokio::test]
async fn test_stacking_one_price_and_one_reward_combine_never_two_price() {
    let price_rule_1_id = Uuid::new_v4();
    let price_rule_2_id = Uuid::new_v4();
    let reward_rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let price_1 = make_active_rule(ActiveRuleSpec {
        rule_id: price_rule_1_id,
        version_id,
        stack_group: "group-price-1",
        priority: 10,
        calculation: json!({ "delta": "0.10" }),
        ..Default::default()
    });

    let price_2 = make_active_rule(ActiveRuleSpec {
        rule_id: price_rule_2_id,
        version_id,
        stack_group: "group-price-2",
        priority: 20,
        calculation: json!({ "delta": "0.20" }),
        ..Default::default()
    });

    let reward = make_active_rule(ActiveRuleSpec {
        rule_id: reward_rule_id,
        version_id,
        stack_group: "group-reward",
        priority: 15,
        benefit_type: "GIFT_VOUCHER",
        benefit_class: BenefitClass::Reward,
        calculation: json!({}),
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![price_1, price_2, reward],
    };

    let mut registry = CalculatorRegistry::new();
    registry.register(Box::new(DummyRewardCalculator));

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(registry),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    // Exactly one PRICE (the higher priority one, price_2) and one REWARD (reward) combine
    assert_eq!(evaluation.snapshot.applied.len(), 2);
    let applied_ids: Vec<Uuid> = evaluation
        .snapshot
        .applied
        .iter()
        .map(|a| a.rule_id)
        .collect();
    assert!(applied_ids.contains(&price_rule_2_id));
    assert!(applied_ids.contains(&reward_rule_id));
    assert!(!applied_ids.contains(&price_rule_1_id));
}

/// §9.1, §4.3: Every unmet condition yields its skip reason.
#[tokio::test]
async fn test_unmet_conditions_yield_skip_reasons() {
    let v_id = Uuid::new_v4();

    // 1. Channel mismatch
    let r_channel = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            channels: Some(vec!["KIOSK".to_string()]),
            ..Default::default()
        },
        ..Default::default()
    });

    // 2. Currency mismatch
    let r_curr = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            currencies: Some(vec!["JPY".to_string()]),
            ..Default::default()
        },
        ..Default::default()
    });

    // 3. Side mismatch
    let r_side = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            shop_side: Some(Side::Sell),
            ..Default::default()
        },
        ..Default::default()
    });

    // 4. Min THB not met
    let r_min = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            min_thb: Some(Decimal::from_str("50000.00").unwrap()),
            ..Default::default()
        },
        ..Default::default()
    });

    // 5. Coupon required but missing
    let r_coup = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            requires_coupon: true,
            ..Default::default()
        },
        ..Default::default()
    });

    // 6. Branch mismatch
    let r_branch = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            branches: Some(vec!["AIRPORT".to_string()]),
            ..Default::default()
        },
        ..Default::default()
    });

    // 7. Referred miss
    let r_referred = make_active_rule(ActiveRuleSpec {
        version_id: v_id,
        stack_group: "g",
        priority: 1,
        eligibility: Eligibility {
            referred: true,
            ..Default::default()
        },
        ..Default::default()
    });

    let active = MockActiveRules {
        rules: vec![
            r_channel.clone(),
            r_curr.clone(),
            r_side.clone(),
            r_min.clone(),
            r_coup.clone(),
            r_branch.clone(),
            r_referred.clone(),
        ],
    };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    // Line THB is 100 * 35.00 = 3500.00 < 50000.00
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(evaluation.snapshot.skipped.len(), 7);
    let reasons: HashMap<Uuid, String> = evaluation
        .snapshot
        .skipped
        .into_iter()
        .map(|s| (s.rule_id, s.reason))
        .collect();

    assert_eq!(
        reasons.get(&r_channel.rule.id).map(String::as_str),
        Some("channelMismatch")
    );
    assert_eq!(
        reasons.get(&r_curr.rule.id).map(String::as_str),
        Some("currencyMismatch")
    );
    assert_eq!(
        reasons.get(&r_side.rule.id).map(String::as_str),
        Some("sideMismatch")
    );
    assert_eq!(
        reasons.get(&r_min.rule.id).map(String::as_str),
        Some("minThbNotMet")
    );
    assert_eq!(
        reasons.get(&r_coup.rule.id).map(String::as_str),
        Some("couponInvalid")
    );
    assert_eq!(
        reasons.get(&r_branch.rule.id).map(String::as_str),
        Some("branchMismatch")
    );
    assert_eq!(
        reasons.get(&r_referred.rule.id).map(String::as_str),
        Some("notReferred")
    );
}

/// §9.1, D-7: Same inputs => same output (pure engine).
#[tokio::test]
async fn test_pure_engine_same_inputs_same_output() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "pure-group",
        priority: 10,
        calculation: json!({ "delta": "0.15" }),
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource::default()),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "0.00",
    )];

    let mut tx1 = test_tx().await;
    let eval_1 = engine.evaluate(&mut tx1, &ctx, &lines).await.unwrap();
    let mut tx2 = test_tx().await;
    let eval_2 = engine.evaluate(&mut tx2, &ctx, &lines).await.unwrap();

    assert_eq!(eval_1, eval_2);

    let json_1 = serde_json::to_string(&eval_1.snapshot).unwrap();
    let json_2 = serde_json::to_string(&eval_2.snapshot).unwrap();
    assert_eq!(json_1, json_2);
}

/// §4.1, §7: BenefitSnapshot JSON structure matches design schema with decimals as strings.
#[tokio::test]
async fn test_snapshot_json_structure() {
    let rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let rule = make_active_rule(ActiveRuleSpec {
        rule_id,
        version_id,
        stack_group: "snap-group",
        priority: 10,
        calculation: json!({ "delta": "0.25" }),
        ..Default::default()
    });

    let active = MockActiveRules { rules: vec![rule] };

    let mut tiers = HashMap::new();
    tiers.insert("cust-1".to_string(), "GOLD".to_string());

    let engine = BenefitEngine::new(
        Arc::new(active),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(MockTierSource { tiers }),
        Arc::new(MockConsentGate::default()),
        Arc::new(MockReferralSource::default()),
    );

    let ctx = make_context(Some("cust-1"), None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "50.00",
    )];

    let mut tx = test_tx().await;
    let evaluation = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();
    let val: serde_json::Value = serde_json::to_value(&evaluation.snapshot).unwrap();

    assert_eq!(val["engineVersion"], 1);
    assert!(val["applied"].is_array());
    assert_eq!(val["applied"][0]["amountThb"], "25.00");
    assert_eq!(val["applied"][0]["rateBefore"], "35.0000");
    assert_eq!(val["applied"][0]["rateAfter"], "35.2500");
    assert_eq!(val["applied"][0]["benefitClass"], "PRICE");
    assert_eq!(val["applied"][0]["floorCapped"], false);
    assert_eq!(val["tierCode"], "GOLD");
}

// ---------------------------------------------------------------------------
// DB-backed tests for PgCampaignRepository ActiveRules impl (Ruling 5, §4.4)
// ---------------------------------------------------------------------------

static SCHEMA_INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ensure_m1_schema(pool: &sqlx::PgPool) {
    SCHEMA_INIT
        .get_or_init(|| async {
            let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
            sqlx::migrate::Migrator::new(migrations.as_path())
                .await
                .expect("read api/migrations")
                .run(pool)
                .await
                .expect("migrations apply");

            // Seed required external references (org B, airport branch, test user)
            sqlx::query(
                "INSERT INTO branches (id, organization_id, name) VALUES ('AIRPORT', '00000000-0000-0000-0000-000000000001', 'XC-Airport') ON CONFLICT (id) DO NOTHING",
            )
            .execute(pool)
            .await
            .expect("seed airport branch");

            sqlx::query(
                "INSERT INTO organizations (id, code, legal_name, timezone, created_at) VALUES ('00000000-0000-0000-0000-000000000002', 'ORG2', 'Org 2', 'Asia/Bangkok', now()) ON CONFLICT (id) DO NOTHING",
            )
            .execute(pool)
            .await
            .expect("seed org B");

            sqlx::query(
                "INSERT INTO users (id, organization_id, name, status, created_by) VALUES ('00000000-0000-0000-0000-000000000099', '00000000-0000-0000-0000-000000000001', 'mk2-tester', 'ACTIVE', 'system') ON CONFLICT (id) DO NOTHING",
            )
            .execute(pool)
            .await
            .expect("seed test user");
        })
        .await;
}

struct SeedVersionSpec {
    pub org_id: Uuid,
    pub campaign_id: Uuid,
    pub version_num: i32,
    pub status: &'static str,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub stack_group: &'static str,
    pub priority: i32,
    pub branch_id: &'static str,
    pub benefit_type: &'static str,
    pub benefit_class: &'static str,
    pub calculation: serde_json::Value,
    pub eligibility: serde_json::Value,
}

impl Default for SeedVersionSpec {
    fn default() -> Self {
        Self {
            org_id: org_id(),
            campaign_id: Uuid::nil(),
            version_num: 1,
            status: "APPROVED",
            starts_at: test_time() - Duration::days(5),
            ends_at: test_time() + Duration::days(5),
            stack_group: "SG_DEFAULT",
            priority: 10,
            branch_id: "MAIN",
            benefit_type: "RATE_IMPROVEMENT",
            benefit_class: "PRICE",
            calculation: json!({ "delta": "0.15" }),
            eligibility: json!({}),
        }
    }
}

struct SeededCampaigns {
    pub active_main_rule_id: Uuid,
    pub active_main_version_id: Uuid,
    pub starts_at_now_rule_id: Uuid,
    pub starts_at_now_version_id: Uuid,
    pub ends_at_now_rule_id: Uuid,
    pub draft_main_rule_id: Uuid,
    pub simulated_main_rule_id: Uuid,
    pub pending_main_rule_id: Uuid,
    pub rejected_main_rule_id: Uuid,
    pub paused_main_rule_id: Uuid,
    pub superseded_main_rule_id: Uuid,
    pub ended_main_rule_id: Uuid,
    pub scheduled_main_rule_id: Uuid,
    pub airport_rule_id: Uuid,
    pub airport_version_id: Uuid,
    pub org_b_rule_id: Uuid,
}

async fn insert_test_version_and_rule(
    tx: &mut Transaction<'_, Postgres>,
    spec: SeedVersionSpec,
) -> (Uuid, Uuid) {
    let version_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();
    let user_id = Uuid::from_str("00000000-0000-0000-0000-000000000099").unwrap();
    let version_hash = match spec.status {
        "DRAFT" | "SIMULATED" => None,
        _ => Some(format!("hash-{}", version_id)),
    };

    sqlx::query(
        r#"
        INSERT INTO campaign_versions (
            id, organization_id, campaign_id, version, status,
            starts_at, ends_at, budget_thb, max_uses, stack_group,
            priority, segment_snapshot_id, version_hash, approval_id,
            simulated_at, edited_at, created_by, created_at
        ) VALUES (
            $1, $2, $3, $4, $5,
            $6, $7, 10000.00, NULL, $8,
            $9, NULL, $10, NULL,
            NULL, $11, $12, $11
        )
        "#,
    )
    .bind(version_id)
    .bind(spec.org_id)
    .bind(spec.campaign_id)
    .bind(spec.version_num)
    .bind(spec.status)
    .bind(spec.starts_at)
    .bind(spec.ends_at)
    .bind(spec.stack_group)
    .bind(spec.priority)
    .bind(version_hash)
    .bind(spec.starts_at)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .expect("insert version");

    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version_id)
        .bind(spec.branch_id)
        .execute(&mut **tx)
        .await
        .expect("insert campaign branch");

    sqlx::query(
        r#"
        INSERT INTO benefit_rules (
            id, campaign_version_id, benefit_type, benefit_class,
            eligibility, calculation, caps, requires_coupon, rule_no
        ) VALUES (
            $1, $2, $3, $4,
            $5, $6, '{}'::jsonb, false, 1
        )
        "#,
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(spec.benefit_type)
    .bind(spec.benefit_class)
    .bind(spec.eligibility)
    .bind(spec.calculation)
    .execute(&mut **tx)
    .await
    .expect("insert benefit rule");

    (version_id, rule_id)
}

async fn seed_test_campaigns(
    tx: &mut Transaction<'_, Postgres>,
    now: DateTime<Utc>,
) -> SeededCampaigns {
    let org_a = org_id();
    let org_b = Uuid::from_str("00000000-0000-0000-0000-000000000002").unwrap();
    let user_id = Uuid::from_str("00000000-0000-0000-0000-000000000099").unwrap();

    let camp_a_id = Uuid::new_v4();
    let camp_b_id = Uuid::new_v4();
    let camp_sim_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(camp_a_id)
    .bind(org_a)
    .bind("Campaign A")
    .bind(user_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .expect("insert campaign A");

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(camp_b_id)
    .bind(org_b)
    .bind("Campaign B")
    .bind(user_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .expect("insert campaign B");

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(camp_sim_id)
    .bind(org_a)
    .bind("Campaign Sim")
    .bind(user_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .expect("insert campaign Sim");

    // 1. ACTIVE in-window version on MAIN (starts_at < now < ends_at, APPROVED) -> INCLUDED
    let (v_active_main, r_active_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 1,
            status: "APPROVED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_ACTIVE_MAIN",
            priority: 100,
            branch_id: "MAIN",
            benefit_type: "RATE_IMPROVEMENT",
            benefit_class: "PRICE",
            calculation: json!({ "delta": "0.15" }),
            eligibility: json!({ "minThb": "100.00" }),
        },
    )
    .await;

    // 2. ACTIVE boundary version on MAIN: starts_at == now (starts_at <= now < ends_at) -> INCLUDED
    let (v_starts_now, r_starts_now) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 2,
            status: "APPROVED",
            starts_at: now,
            ends_at: now + Duration::days(5),
            stack_group: "SG_STARTS_NOW",
            priority: 90,
            branch_id: "MAIN",
            benefit_type: "RATE_IMPROVEMENT",
            benefit_class: "PRICE",
            calculation: json!({ "delta": "0.10" }),
            eligibility: json!({}),
        },
    )
    .await;

    // 3. Boundary version on MAIN: ends_at == now (must be excluded because ends_at > now is false)
    let (_v_ends_now, r_ends_now) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 3,
            status: "APPROVED",
            starts_at: now - Duration::days(5),
            ends_at: now,
            stack_group: "SG_ENDS_NOW",
            priority: 85,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 4. DRAFT version on MAIN (must be excluded)
    let (_v_draft_main, r_draft_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 4,
            status: "DRAFT",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_DRAFT",
            priority: 80,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 5. SIMULATED version on MAIN (must be excluded, own campaign for one_draft_per_campaign constraint)
    let (_v_sim_main, r_sim_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_sim_id,
            version_num: 1,
            status: "SIMULATED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_SIMULATED",
            priority: 75,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 6. PENDING_APPROVAL version on MAIN (must be excluded)
    let (_v_pending_main, r_pending_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 6,
            status: "PENDING_APPROVAL",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_PENDING",
            priority: 70,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 7. REJECTED version on MAIN (must be excluded)
    let (_v_rej_main, r_rej_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 7,
            status: "REJECTED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_REJECTED",
            priority: 65,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 8. PAUSED version on MAIN (must be excluded)
    let (_v_paused_main, r_paused_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 8,
            status: "PAUSED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_PAUSED",
            priority: 60,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 9. SUPERSEDED version on MAIN (must be excluded)
    let (_v_sup_main, r_sup_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 9,
            status: "SUPERSEDED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_SUPERSEDED",
            priority: 55,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 10. ENDED / expired version on MAIN (ends_at < now, must be excluded)
    let (_v_ended_main, r_ended_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 10,
            status: "APPROVED",
            starts_at: now - Duration::days(10),
            ends_at: now - Duration::days(1),
            stack_group: "SG_ENDED",
            priority: 50,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 11. SCHEDULED / future version on MAIN (starts_at > now, must be excluded)
    let (_v_sched_main, r_sched_main) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 11,
            status: "APPROVED",
            starts_at: now + Duration::days(1),
            ends_at: now + Duration::days(10),
            stack_group: "SG_SCHEDULED",
            priority: 40,
            branch_id: "MAIN",
            ..Default::default()
        },
    )
    .await;

    // 12. ACTIVE in-window version on another branch: AIRPORT (must be excluded when querying MAIN)
    let (v_airport, r_airport) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_a,
            campaign_id: camp_a_id,
            version_num: 12,
            status: "APPROVED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_AIRPORT",
            priority: 30,
            branch_id: "AIRPORT",
            benefit_type: "FEE_WAIVER",
            benefit_class: "PRICE",
            calculation: json!({ "percent": "1.00" }),
            eligibility: json!({}),
        },
    )
    .await;

    // 13. ACTIVE in-window version for another organisation: Org B on MAIN (must be excluded when querying Org A)
    let (_v_org_b, r_org_b) = insert_test_version_and_rule(
        tx,
        SeedVersionSpec {
            org_id: org_b,
            campaign_id: camp_b_id,
            version_num: 1,
            status: "APPROVED",
            starts_at: now - Duration::days(5),
            ends_at: now + Duration::days(5),
            stack_group: "SG_ORG_B",
            priority: 20,
            branch_id: "MAIN",
            benefit_type: "RATE_IMPROVEMENT",
            benefit_class: "PRICE",
            calculation: json!({ "delta": "0.80" }),
            eligibility: json!({}),
        },
    )
    .await;

    SeededCampaigns {
        active_main_rule_id: r_active_main,
        active_main_version_id: v_active_main,
        starts_at_now_rule_id: r_starts_now,
        starts_at_now_version_id: v_starts_now,
        ends_at_now_rule_id: r_ends_now,
        draft_main_rule_id: r_draft_main,
        simulated_main_rule_id: r_sim_main,
        pending_main_rule_id: r_pending_main,
        rejected_main_rule_id: r_rej_main,
        paused_main_rule_id: r_paused_main,
        superseded_main_rule_id: r_sup_main,
        ended_main_rule_id: r_ended_main,
        scheduled_main_rule_id: r_sched_main,
        airport_rule_id: r_airport,
        airport_version_id: v_airport,
        org_b_rule_id: r_org_b,
    }
}

/// Design §4.4 row "only ACTIVE versions are evaluated", ruling 5 (#1392):
/// `PgCampaignRepository::active_for` returns only APPROVED versions inside their validity window
/// (`starts_at <= now < ends_at`), filtered by branch, ordered by stack_group / priority.
///
/// Verifies against seeded database:
/// - an ACTIVE in-window version on MAIN is returned, with stack_group, priority, version_id and rule fields asserted;
/// - boundary starts_at == now is included; boundary ends_at == now is excluded;
/// - DRAFT, SIMULATED, PENDING_APPROVAL, REJECTED, PAUSED, and SUPERSEDED versions are excluded;
/// - out-of-window versions are excluded, both before (SCHEDULED) and after (ENDED);
/// - another branch's version is excluded;
/// - another organisation's version is excluded.
#[tokio::test]
async fn test_pg_campaign_repository_active_for_branch_active_versions_only() {
    let pool = test_pool().await;
    ensure_m1_schema(pool).await;

    let mut tx = pool.begin().await.expect("begin");
    let now = test_time();
    let dataset = seed_test_campaigns(&mut tx, now).await;

    let repo = PgCampaignRepository::new();
    let org = org_id();

    let rules = repo
        .active_for(&mut tx, org, "MAIN", now)
        .await
        .expect("active_for query succeeds");

    // Exact returned set must contain exactly 2 active rules for org A on branch MAIN:
    // - active_main (priority 100)
    // - starts_at_now (priority 90, starts_at == now boundary included)
    assert_eq!(
        rules.len(),
        2,
        "expected exactly 2 active rules returned for org A on branch MAIN, got {}",
        rules.len()
    );

    let by_version: std::collections::HashMap<Uuid, &ActiveRule> =
        rules.iter().map(|r| (r.version_id, r)).collect();

    let active_rule = by_version
        .get(&dataset.active_main_version_id)
        .expect("active_main version present");
    assert_eq!(active_rule.rule.id, dataset.active_main_rule_id);
    assert_eq!(active_rule.stack_group, "SG_ACTIVE_MAIN");
    assert_eq!(active_rule.priority, 100);
    assert_eq!(active_rule.rule.benefit_type, "RATE_IMPROVEMENT");
    assert_eq!(active_rule.rule.benefit_class, BenefitClass::Price);
    assert_eq!(active_rule.rule.calculation, json!({ "delta": "0.15" }));
    assert_eq!(
        active_rule.rule.eligibility.min_thb,
        Some(Decimal::from_str("100.00").unwrap())
    );

    let boundary_rule = by_version
        .get(&dataset.starts_at_now_version_id)
        .expect("starts_at_now boundary version present");
    assert_eq!(boundary_rule.rule.id, dataset.starts_at_now_rule_id);
    assert_eq!(boundary_rule.stack_group, "SG_STARTS_NOW");
    assert_eq!(boundary_rule.priority, 90);
    assert_eq!(boundary_rule.rule.benefit_type, "RATE_IMPROVEMENT");
    assert_eq!(boundary_rule.rule.benefit_class, BenefitClass::Price);

    // Verify all excluded rule IDs are NOT returned
    let returned_rule_ids: Vec<Uuid> = rules.iter().map(|r| r.rule.id).collect();
    assert!(
        !returned_rule_ids.contains(&dataset.ends_at_now_rule_id),
        "ends_at == now must be excluded (boundary: starts_at <= now < ends_at)"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.draft_main_rule_id),
        "DRAFT version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.simulated_main_rule_id),
        "SIMULATED version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.pending_main_rule_id),
        "PENDING_APPROVAL version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.rejected_main_rule_id),
        "REJECTED version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.paused_main_rule_id),
        "PAUSED version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.superseded_main_rule_id),
        "SUPERSEDED version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.ended_main_rule_id),
        "ENDED/expired version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.scheduled_main_rule_id),
        "SCHEDULED/future version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.airport_rule_id),
        "Different branch version must be excluded"
    );
    assert!(
        !returned_rule_ids.contains(&dataset.org_b_rule_id),
        "Different organisation version must be excluded"
    );
}

/// Branch filtering against seeded DB:
/// - querying "AIRPORT" returns AIRPORT's active rule, excluding MAIN and others;
/// - querying a non-existent branch returns empty against a populated DB.
#[tokio::test]
async fn test_pg_campaign_repository_active_for_different_branch() {
    let pool = test_pool().await;
    ensure_m1_schema(pool).await;

    let mut tx = pool.begin().await.expect("begin");
    let now = test_time();
    let dataset = seed_test_campaigns(&mut tx, now).await;

    let repo = PgCampaignRepository::new();
    let org = org_id();

    // Query for branch AIRPORT
    let rules_airport = repo
        .active_for(&mut tx, org, "AIRPORT", now)
        .await
        .expect("active_for query succeeds for AIRPORT");

    assert_eq!(
        rules_airport.len(),
        1,
        "expected exactly 1 active rule returned for org A on branch AIRPORT, got {}",
        rules_airport.len()
    );
    assert_eq!(rules_airport[0].rule.id, dataset.airport_rule_id);
    assert_eq!(rules_airport[0].version_id, dataset.airport_version_id);
    assert_eq!(rules_airport[0].stack_group, "SG_AIRPORT");
    assert_eq!(rules_airport[0].priority, 30);
    assert_eq!(rules_airport[0].rule.benefit_type, "FEE_WAIVER");

    // Query for a branch with no active campaign against populated DB
    let rules_none = repo
        .active_for(&mut tx, org, "NON_EXISTENT_BRANCH", now)
        .await
        .expect("active_for query succeeds for NON_EXISTENT_BRANCH");

    assert!(
        rules_none.is_empty(),
        "querying a non-existent branch against populated DB must return empty, got {}",
        rules_none.len()
    );
}
