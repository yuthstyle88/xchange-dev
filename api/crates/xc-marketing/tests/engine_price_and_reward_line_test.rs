//! #2656: A quote line with a PRICE and a REWARD benefit must retain the
//! improved rate / waived fee from the PRICE benefit, and not have them overwritten
//! by the REWARD pass with the base values.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xc_marketing::domain::benefit::calculator::{BenefitCalculator, BenefitEffect};
use xc_marketing::domain::benefit::eligibility::Eligibility;
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};
use xc_marketing::domain::engine::active_rules::{ActiveRule, ActiveRules};
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::engine::engine::{BenefitEngine, EvalContext, QuoteLineInput};
use xc_marketing::domain::engine::referral_source::ReferralSource;
use xc_marketing::domain::engine::tier_source::TierSource;

// ---------------------------------------------------------------------------
// DB test harness (as benefit_engine_test.rs)
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
struct MockTierSource;

impl TierSource for MockTierSource {
    fn tier_of(&self, _org: Uuid, _customer: &str) -> Option<String> {
        None
    }
}

#[derive(Clone, Default)]
struct MockConsentGate;

impl ConsentGate for MockConsentGate {
    fn marketing_allowed(&self, _customer: &str) -> bool {
        true
    }
}

#[derive(Clone, Default)]
struct MockReferralSource;

impl ReferralSource for MockReferralSource {
    fn referred(&self, _customer: &str, _at: DateTime<Utc>) -> bool {
        false
    }
}

// Dummy REWARD calculator for stacking tests (benefit_engine_test.rs:154-180)
struct DummyRewardCalculator;
impl BenefitCalculator for DummyRewardCalculator {
    fn kind(&self) -> &str {
        "GIFT_VOUCHER"
    }
    fn class(&self) -> BenefitClass {
        BenefitClass::Reward
    }
    fn validate_params(
        &self,
        _params: &serde_json::Value,
    ) -> Result<(), xc_marketing::domain::benefit::calculator::ParamError> {
        Ok(())
    }
    fn apply(
        &self,
        _line: &QuoteLineInput,
        _params: &serde_json::Value,
    ) -> Result<BenefitEffect, xc_marketing::domain::benefit::calculator::ParamError> {
        Ok(BenefitEffect {
            new_final_rate: None,
            new_fee_thb: None,
            amount_thb: Money::from_minor(10000).unwrap(), // 100.00 THB
        })
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

fn build_engine(rules: Vec<ActiveRule>) -> BenefitEngine {
    let active = MockActiveRules { rules };
    let mut registry = CalculatorRegistry::new();
    registry.register(Box::new(DummyRewardCalculator));

    BenefitEngine::new(
        Arc::new(active),
        Arc::new(registry),
        Arc::new(MockTierSource),
        Arc::new(MockConsentGate),
        Arc::new(MockReferralSource),
    )
}

// ---------------------------------------------------------------------------
// Behaviour Table Tests (Issue #2656 §9)
// ---------------------------------------------------------------------------

/// P1: one PRICE RATE_IMPROVEMENT ({"delta":"0.20"}), no REWARD
/// -> the improved rate R (≠ 35.00, as today); fee unchanged (keep); the PRICE rule only
#[tokio::test]
async fn test_p1_one_price_rate_improvement_no_reward() {
    let price_rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let price_rule = make_active_rule(ActiveRuleSpec {
        rule_id: price_rule_id,
        version_id,
        stack_group: "group-price",
        priority: 10,
        benefit_type: "RATE_IMPROVEMENT",
        benefit_class: BenefitClass::Price,
        calculation: json!({ "delta": "0.20" }),
        ..Default::default()
    });

    let engine = build_engine(vec![price_rule]);
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
    let eval = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(eval.snapshot.applied.len(), 1);
    assert_eq!(eval.snapshot.applied[0].rule_id, price_rule_id);

    let improved_rate = eval.lines[0].final_rate;
    assert_ne!(
        improved_rate,
        Rate::parse("35.00").unwrap(),
        "improved rate must differ from base rate 35.00"
    );
    assert_eq!(
        improved_rate,
        Rate::parse("35.20").unwrap(),
        "improved rate R must be 35.20"
    );
    assert_eq!(eval.lines[0].fee_thb, Money::parse("0.00").unwrap());
}

/// P2: the same PRICE RATE_IMPROVEMENT and one REWARD (DummyRewardCalculator)
/// -> R, the same rate as P1 (main: 35.00); fee unchanged; both rules; the REWARD's rate_after = rate_before = 35.00
#[tokio::test]
async fn test_p2_price_rate_improvement_and_reward_retains_price_benefit() {
    let price_rule_id = Uuid::new_v4();
    let reward_rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let price_rule = make_active_rule(ActiveRuleSpec {
        rule_id: price_rule_id,
        version_id,
        stack_group: "group-price",
        priority: 10,
        benefit_type: "RATE_IMPROVEMENT",
        benefit_class: BenefitClass::Price,
        calculation: json!({ "delta": "0.20" }),
        ..Default::default()
    });

    let reward_rule = make_active_rule(ActiveRuleSpec {
        rule_id: reward_rule_id,
        version_id,
        stack_group: "group-reward",
        priority: 10,
        benefit_type: "GIFT_VOUCHER",
        benefit_class: BenefitClass::Reward,
        calculation: json!({}),
        ..Default::default()
    });

    let engine = build_engine(vec![price_rule, reward_rule]);
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
    let eval = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(eval.snapshot.applied.len(), 2);
    let applied_ids: Vec<Uuid> = eval.snapshot.applied.iter().map(|a| a.rule_id).collect();
    assert!(applied_ids.contains(&price_rule_id));
    assert!(applied_ids.contains(&reward_rule_id));

    let reward_applied = eval
        .snapshot
        .applied
        .iter()
        .find(|a| a.rule_id == reward_rule_id)
        .expect("reward applied benefit");
    assert_eq!(reward_applied.rate_before, lines[0].base_rate.to_string());
    assert_eq!(reward_applied.rate_after, lines[0].base_rate.to_string());

    assert_eq!(
        eval.lines[0].final_rate,
        Rate::parse("35.20").unwrap(),
        "line final_rate must retain R (35.20), not reset to base (35.00)"
    );
    assert_eq!(eval.lines[0].fee_thb, Money::parse("0.00").unwrap());
}

/// P3: one PRICE FEE_WAIVER ({"percent":"1.00"}) on a line with fee 50.00 and one REWARD
/// -> rate unchanged; fee 0.00 (main: 50.00); both rules
#[tokio::test]
async fn test_p3_price_fee_waiver_and_reward_retains_waived_fee() {
    let price_rule_id = Uuid::new_v4();
    let reward_rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let price_rule = make_active_rule(ActiveRuleSpec {
        rule_id: price_rule_id,
        version_id,
        stack_group: "group-price",
        priority: 10,
        benefit_type: "FEE_WAIVER",
        benefit_class: BenefitClass::Price,
        calculation: json!({ "percent": "1.00" }),
        ..Default::default()
    });

    let reward_rule = make_active_rule(ActiveRuleSpec {
        rule_id: reward_rule_id,
        version_id,
        stack_group: "group-reward",
        priority: 10,
        benefit_type: "GIFT_VOUCHER",
        benefit_class: BenefitClass::Reward,
        calculation: json!({}),
        ..Default::default()
    });

    let engine = build_engine(vec![price_rule, reward_rule]);
    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "50.00",
    )];

    let mut tx = test_tx().await;
    let eval = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(eval.snapshot.applied.len(), 2);
    let applied_ids: Vec<Uuid> = eval.snapshot.applied.iter().map(|a| a.rule_id).collect();
    assert!(applied_ids.contains(&price_rule_id));
    assert!(applied_ids.contains(&reward_rule_id));

    assert_eq!(
        eval.lines[0].final_rate,
        Rate::parse("35.00").unwrap(),
        "rate remains base rate 35.00"
    );
    assert_eq!(
        eval.lines[0].fee_thb,
        Money::parse("0.00").unwrap(),
        "fee must be waived to 0.00, not reset to base 50.00"
    );
}

/// P4: one REWARD only
/// -> base rate and base fee (keep); the REWARD rule only
#[tokio::test]
async fn test_p4_one_reward_only_keeps_base_rate_and_fee() {
    let reward_rule_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();

    let reward_rule = make_active_rule(ActiveRuleSpec {
        rule_id: reward_rule_id,
        version_id,
        stack_group: "group-reward",
        priority: 10,
        benefit_type: "GIFT_VOUCHER",
        benefit_class: BenefitClass::Reward,
        calculation: json!({}),
        ..Default::default()
    });

    let engine = build_engine(vec![reward_rule]);
    let ctx = make_context(None, None, None, vec![]);
    let lines = vec![make_line(
        Side::Buy,
        "35.00",
        "35.00",
        "36.00",
        "100",
        "50.00",
    )];

    let mut tx = test_tx().await;
    let eval = engine.evaluate(&mut tx, &ctx, &lines).await.unwrap();

    assert_eq!(eval.snapshot.applied.len(), 1);
    assert_eq!(eval.snapshot.applied[0].rule_id, reward_rule_id);

    assert_eq!(eval.lines[0].final_rate, Rate::parse("35.00").unwrap());
    assert_eq!(eval.lines[0].fee_thb, Money::parse("50.00").unwrap());
}
