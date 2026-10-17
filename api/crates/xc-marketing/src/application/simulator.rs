//! `CampaignSimulator` (MK-4, design `marketing-campaigns.md` §4.1, §6, §7, §10, D-4, D-7).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

use crate::domain::benefit::eligibility::Eligibility;
use crate::domain::benefit::registry::CalculatorRegistry;
use crate::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};
use crate::domain::engine::active_rules::{ActiveRule, ActiveRules};
use crate::domain::engine::consent_gate::ConsentGate;
use crate::domain::engine::engine::{BenefitEngine, EvalContext, Evaluation, QuoteLineInput};
use crate::domain::engine::referral_source::ReferralSource;
use crate::domain::engine::tier_source::TierSource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationScenario {
    pub branch_id: String,
    pub currency: String,
    pub amount: Decimal,
}

#[derive(Debug, Clone)]
pub struct SimulationResult {
    pub simulation_id: Uuid,
    pub version_id: Uuid,
    pub cost_source: String,
    pub evaluation: Option<Evaluation>,
    pub simulated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SimulationError {
    #[error("versionNotFound")]
    VersionNotFound,
    #[error("rateUnavailable: no published line")]
    RateUnavailable,
    #[error("database: {0}")]
    Database(String),
}

struct DefaultTierSource;
impl TierSource for DefaultTierSource {
    fn tier_of(&self, _organization_id: Uuid, _customer: &str) -> Option<String> {
        None
    }
}

struct DefaultConsentGate;
impl ConsentGate for DefaultConsentGate {
    fn marketing_allowed(&self, _customer: &str) -> bool {
        true
    }
}

struct DefaultReferralSource;
impl ReferralSource for DefaultReferralSource {
    fn referred(&self, _customer: &str, _at: DateTime<Utc>) -> bool {
        false
    }
}

struct SimulatorActiveRules {
    version_id: Uuid,
}

impl ActiveRules for SimulatorActiveRules {
    fn active_for<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        branch: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ActiveRule>>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT \
                    r.id AS rule_id, \
                    r.campaign_version_id, \
                    r.benefit_type, \
                    r.benefit_class, \
                    r.eligibility, \
                    r.calculation, \
                    r.caps, \
                    r.requires_coupon, \
                    r.rule_no, \
                    v.stack_group, \
                    v.priority, \
                    v.id AS version_id, \
                    v.created_at AS version_created_at, \
                    v.segment_snapshot_id \
                 FROM campaign_versions v \
                 JOIN campaign_branches b ON b.campaign_version_id = v.id \
                 JOIN benefit_rules r ON r.campaign_version_id = v.id \
                 WHERE v.organization_id = $1 \
                   AND (v.id = $2 OR (v.status = 'APPROVED' AND v.starts_at <= $3 AND v.ends_at > $3)) \
                   AND b.branch_id = $4 \
                 ORDER BY v.stack_group ASC, v.priority DESC, r.rule_no ASC",
            )
            .bind(organization_id)
            .bind(self.version_id)
            .bind(now)
            .bind(branch)
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

            let mut result = Vec::with_capacity(rows.len());

            for row in rows {
                let benefit_class_str: String = row.get("benefit_class");
                let benefit_class = match benefit_class_str.as_str() {
                    "PRICE" => BenefitClass::Price,
                    "REWARD" => BenefitClass::Reward,
                    other => {
                        return Err(pos_error::AppError::Internal(format!(
                            "unknown benefit class: {other}"
                        )));
                    }
                };

                let eligibility_val: serde_json::Value = row.get("eligibility");
                let eligibility: Eligibility =
                    serde_json::from_value(eligibility_val).map_err(|e| {
                        pos_error::AppError::Internal(format!("failed to decode eligibility: {e}"))
                    })?;

                let calculation: serde_json::Value = row.get("calculation");

                let caps_val: serde_json::Value = row.get("caps");
                let caps: BenefitCaps = serde_json::from_value(caps_val).map_err(|e| {
                    pos_error::AppError::Internal(format!("failed to decode caps: {e}"))
                })?;

                let rule = BenefitRule {
                    id: row.get("rule_id"),
                    campaign_version_id: row.get("campaign_version_id"),
                    benefit_type: row.get("benefit_type"),
                    benefit_class,
                    eligibility,
                    calculation,
                    caps,
                    requires_coupon: row.get("requires_coupon"),
                    rule_no: row.get("rule_no"),
                };

                result.push(ActiveRule {
                    rule,
                    stack_group: row.get("stack_group"),
                    priority: row.get("priority"),
                    version_id: row.get("version_id"),
                    version_created_at: row.get("version_created_at"),
                    segment_snapshot_id: row.get("segment_snapshot_id"),
                });
            }

            Ok(result)
        })
    }
}

pub struct CampaignSimulator {
    pool: PgPool,
}

impl CampaignSimulator {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn simulate_in(
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version_id: Uuid,
        scenarios: Vec<SimulationScenario>,
        simulated_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<SimulationResult, SimulationError> {
        let version_exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM campaign_versions WHERE organization_id = $1 AND id = $2",
        )
        .bind(organization_id)
        .bind(version_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| SimulationError::Database(e.to_string()))?;

        if version_exists.is_none() {
            return Err(SimulationError::VersionNotFound);
        }

        // Check ledger for completed transactions with realized margin (D-4, §6.1)
        let has_ledger: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 \
                 FROM transaction_lines tl \
                 JOIN transactions t ON t.id = tl.transaction_id \
                 WHERE tl.organization_id = $1 \
                   AND t.status = 'COMPLETED' \
                   AND tl.realized_margin_thb IS NOT NULL \
             )",
        )
        .bind(organization_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| SimulationError::Database(e.to_string()))?;

        let cost_source = if has_ledger {
            "LEDGER".to_string()
        } else {
            "UNAVAILABLE".to_string()
        };

        let evaluation = if scenarios.is_empty() {
            None
        } else {
            let mut eval_lines = Vec::new();
            let mut branch = "MAIN".to_string();

            for s in &scenarios {
                branch = s.branch_id.clone();

                let rate_row = sqlx::query(
                    "SELECT prl.buy_rate, prl.sell_rate, prl.unit_size \
                     FROM published_rate_lines prl \
                     JOIN branches b ON b.current_publication_id = prl.publication_id \
                     WHERE b.id = $1 AND prl.currency_code = $2 AND prl.tier_lower_exclusive_thb IS NULL \
                     ORDER BY prl.unit_size ASC LIMIT 1",
                )
                .bind(&s.branch_id)
                .bind(&s.currency)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| SimulationError::Database(e.to_string()))?;

                let Some(r) = rate_row else {
                    return Err(SimulationError::RateUnavailable);
                };
                let buy_opt: Option<Decimal> = r.get("buy_rate");
                let Some(buy_dec) = buy_opt else {
                    return Err(SimulationError::RateUnavailable);
                };
                let sell_opt: Option<Decimal> = r.get("sell_rate");
                let sell_dec = sell_opt.unwrap_or(buy_dec);
                let u_size: Decimal = r.get("unit_size");
                let (buy_rate_str, sell_rate_str, unit_size_val) = (
                    buy_dec.to_string(),
                    sell_dec.to_string(),
                    u_size.to_string(),
                );

                let curr = CurrencyCode::parse(&s.currency)
                    .map_err(|e| SimulationError::Database(format!("invalid currency: {e}")))?;
                let base_rate = Rate::parse(&buy_rate_str)
                    .map_err(|e| SimulationError::Database(format!("invalid buy rate: {e}")))?;
                let sell_rate = Rate::parse(&sell_rate_str)
                    .map_err(|e| SimulationError::Database(format!("invalid sell rate: {e}")))?;
                let unit_size = UnitSize::parse(&unit_size_val)
                    .map_err(|e| SimulationError::Database(format!("invalid unit size: {e}")))?;

                eval_lines.push(QuoteLineInput {
                    currency: curr,
                    shop_side: Side::Buy,
                    quantity: ForeignAmount::from_decimal(s.amount),
                    unit_size,
                    base_rate,
                    fee_thb: Money::ZERO,
                    published_buy_rate: base_rate,
                    published_sell_rate: sell_rate,
                });
            }

            let ctx = EvalContext {
                organization_id,
                branch,
                channel: "DESK".to_string(),
                customer_id: None,
                coupon_code: None,
                now,
                max_spread_share: None,
                segment_memberships: vec![],
            };

            let engine = BenefitEngine::new(
                Arc::new(SimulatorActiveRules { version_id }),
                Arc::new(CalculatorRegistry::new()),
                Arc::new(DefaultTierSource),
                Arc::new(DefaultConsentGate),
                Arc::new(DefaultReferralSource),
            );

            let eval = engine
                .evaluate(tx, &ctx, &eval_lines)
                .await
                .map_err(|e| SimulationError::Database(e.to_string()))?;

            Some(eval)
        };

        let simulation_id = Uuid::new_v4();
        let input_json = serde_json::to_value(&scenarios)
            .map_err(|e| SimulationError::Database(e.to_string()))?;
        let result_json = match &evaluation {
            Some(ev) => serde_json::to_value(&ev.snapshot)
                .map_err(|e| SimulationError::Database(e.to_string()))?,
            None => serde_json::json!({}),
        };

        sqlx::query(
            "INSERT INTO campaign_simulations ( \
                 id, campaign_version_id, input, result, cost_source, simulated_by, simulated_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(simulation_id)
        .bind(version_id)
        .bind(input_json)
        .bind(result_json)
        .bind(&cost_source)
        .bind(simulated_by)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(|e| SimulationError::Database(e.to_string()))?;

        sqlx::query(
            "UPDATE campaign_versions SET \
                 simulated_at = $1, \
                 status = CASE WHEN status = 'DRAFT' THEN 'SIMULATED' ELSE status END \
             WHERE id = $2",
        )
        .bind(now)
        .bind(version_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| SimulationError::Database(e.to_string()))?;

        Ok(SimulationResult {
            simulation_id,
            version_id,
            cost_source,
            evaluation,
            simulated_at: now,
        })
    }

    pub async fn simulate(
        &self,
        organization_id: Uuid,
        version_id: Uuid,
        scenarios: Vec<SimulationScenario>,
        simulated_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<SimulationResult, SimulationError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| SimulationError::Database(e.to_string()))?;
        let res = Self::simulate_in(
            &mut tx,
            organization_id,
            version_id,
            scenarios,
            simulated_by,
            now,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| SimulationError::Database(e.to_string()))?;
        Ok(res)
    }
}
