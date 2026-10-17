//! Up-front tests for the K-a caller migration of `BenefitEngine::evaluate` (#2063, ticket E),
//! written by the ticket author (claude/d7aa64) before the fix. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! `domain/engine/engine.rs:124` prices every quote line with `ExchangeArithmetic::thb`
//! (quantity × base_rate ÷ unit_size) to give the eligibility check its `thb_amount`; the `*` and
//! `/` there panic when the value leaves `Decimal` (≈7.9e28). Per the ticket author's ruling the
//! engine is tested on its own: a line of 1e18 at 1e8 per 0.0001 is 1e30.
//!
//! Required: the checked `ExchangeArithmetic::try_thb` (#2064), its overflow answered as an
//! `Err` of `evaluate`'s `AppResult` — never a panic. The simulator keeps mapping an engine error
//! to its 500. Ordinary lines evaluate as today (the `min_thb` eligibility reads the same
//! `thb_amount`).
//!
//! The engine only needs a transaction for `ActiveRules`, which is a test double here; nothing is
//! read from or written to the database. Needs `TEST_DATABASE_URL`; nothing here skips.

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
use xc_marketing::domain::benefit::eligibility::Eligibility;
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};
use xc_marketing::domain::engine::active_rules::{ActiveRule, ActiveRules};
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::engine::engine::{BenefitEngine, EvalContext, QuoteLineInput};
use xc_marketing::domain::engine::referral_source::ReferralSource;
use xc_marketing::domain::engine::tier_source::TierSource;

// ── DB harness (as `benefit_engine_test.rs`) ──────────────────────────────────

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
                    .max_connections(4)
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

// ── test doubles ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct FixedRules {
    rules: Vec<ActiveRule>,
}

impl ActiveRules for FixedRules {
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

struct NoTier;
impl TierSource for NoTier {
    fn tier_of(&self, _org: Uuid, _customer: &str) -> Option<String> {
        None
    }
}

struct NoConsent;
impl ConsentGate for NoConsent {
    fn marketing_allowed(&self, _customer: &str) -> bool {
        false
    }
}

struct NotReferred;
impl ReferralSource for NotReferred {
    fn referred(&self, _customer: &str, _at: DateTime<Utc>) -> bool {
        false
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn engine(rules: Vec<ActiveRule>) -> BenefitEngine {
    BenefitEngine::new(
        Arc::new(FixedRules { rules }),
        Arc::new(CalculatorRegistry::new()),
        Arc::new(NoTier),
        Arc::new(NoConsent),
        Arc::new(NotReferred),
    )
}

fn context() -> EvalContext {
    EvalContext {
        organization_id: Uuid::from_str("00000000-0000-0000-0000-000000000001").unwrap(),
        branch: "MAIN".to_string(),
        channel: "DESK".to_string(),
        customer_id: None,
        coupon_code: None,
        now: Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap(),
        max_spread_share: None,
        segment_memberships: vec![],
    }
}

/// A BUY line of `qty` USD at `rate` per `unit` (published buy `rate`, sell `sell`), fee 0.
fn line(qty: &str, rate: &str, sell: &str, unit: &str) -> QuoteLineInput {
    QuoteLineInput {
        currency: CurrencyCode::parse("USD").unwrap(),
        shop_side: Side::Buy,
        quantity: ForeignAmount::from_decimal(Decimal::from_str(qty).unwrap()),
        unit_size: UnitSize::parse(unit).unwrap(),
        base_rate: Rate::parse(rate).unwrap(),
        fee_thb: Money::parse("0").unwrap(),
        published_buy_rate: Rate::parse(rate).unwrap(),
        published_sell_rate: Rate::parse(sell).unwrap(),
    }
}

/// A RATE_IMPROVEMENT of 0.10 for lines worth at least `min_thb`.
fn min_thb_rule(rule_id: Uuid, min_thb: &str) -> ActiveRule {
    let version_id = Uuid::new_v4();
    ActiveRule {
        rule: BenefitRule {
            id: rule_id,
            campaign_version_id: version_id,
            benefit_type: "RATE_IMPROVEMENT".to_string(),
            benefit_class: BenefitClass::Price,
            eligibility: Eligibility {
                min_thb: Some(Decimal::from_str(min_thb).unwrap()),
                ..Eligibility::default()
            },
            calculation: json!({ "delta": "0.10" }),
            caps: BenefitCaps::default(),
            requires_coupon: false,
            rule_no: 1,
        },
        stack_group: "default-group".to_string(),
        priority: 10,
        version_id,
        version_created_at: Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap(),
        segment_snapshot_id: None,
    }
}

// ── engine.rs:124 ─────────────────────────────────────────────────────────────

/// 1e18 × 1e8 ÷ 0.0001 = 1e30, past `Decimal::MAX`: an `Err`, not a panic.
#[tokio::test]
async fn a_line_past_decimal_is_an_err_not_a_panic() {
    let lines = vec![line(
        "1000000000000000000",
        "100000000",
        "100000000",
        "0.0001",
    )];
    let mut tx = test_tx().await;

    let got = engine(vec![]).evaluate(&mut tx, &context(), &lines).await;

    assert!(got.is_err(), "expected Err, got {got:?}");
}

/// The overflowing line is not hidden behind an ordinary one before it.
#[tokio::test]
async fn a_line_past_decimal_after_an_ordinary_line_is_an_err() {
    let lines = vec![
        line("100", "35", "36", "1"),
        line("1000000000000000000", "100000000", "100000000", "0.0001"),
    ];
    let mut tx = test_tx().await;

    let got = engine(vec![]).evaluate(&mut tx, &context(), &lines).await;

    assert!(got.is_err(), "expected Err, got {got:?}");
}

/// Keep-pin: the line's THB still decides `min_thb` exactly as today — 100 × 35 = 3,500.00
/// meets 3,000 (applied, 35 → 35.10), 50 × 35 = 1,750.00 does not (skipped `minThbNotMet`).
#[tokio::test]
async fn ordinary_lines_keep_their_thb_for_min_thb() {
    let rule_id = Uuid::new_v4();
    let mut tx = test_tx().await;

    let met = engine(vec![min_thb_rule(rule_id, "3000")])
        .evaluate(&mut tx, &context(), &[line("100", "35", "36", "1")])
        .await
        .expect("an ordinary line evaluates");
    assert_eq!(met.snapshot.applied.len(), 1, "{met:?}");
    assert_eq!(met.snapshot.applied[0].rule_id, rule_id);
    assert_eq!(met.lines[0].final_rate, Rate::parse("35.10").unwrap());

    let missed = engine(vec![min_thb_rule(rule_id, "3000")])
        .evaluate(&mut tx, &context(), &[line("50", "35", "36", "1")])
        .await
        .expect("an ordinary line evaluates");
    assert!(missed.snapshot.applied.is_empty(), "{missed:?}");
    assert_eq!(missed.snapshot.skipped.len(), 1, "{missed:?}");
    assert_eq!(missed.snapshot.skipped[0].reason, "minThbNotMet");
    assert_eq!(missed.lines[0].final_rate, Rate::parse("35").unwrap());

    // The edge: 1,500 at 1 per 0.5 = 3,000.00 exactly, meets 3,000 (unit_size divides).
    let edge = engine(vec![min_thb_rule(rule_id, "3000")])
        .evaluate(&mut tx, &context(), &[line("1500", "1", "2", "0.5")])
        .await
        .expect("an ordinary line evaluates");
    assert_eq!(edge.snapshot.applied.len(), 1, "{edge:?}");
}
