use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::money::{
    CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Rate, Side, UnitSize,
};

use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use super::active_rules::ActiveRules;
use super::consent_gate::ConsentGate;
use super::rate_floor::RateFloor;
use super::referral_source::ReferralSource;
use super::snapshot::{AppliedBenefit, BenefitSnapshot, SkippedRule};
use super::stacking::Stacking;
use super::tier_source::TierSource;
use crate::domain::benefit::eligibility::EligibilityContext;
use crate::domain::benefit::registry::CalculatorRegistry;

#[derive(Debug, Clone, PartialEq)]
pub struct EvalContext {
    pub organization_id: Uuid,
    pub branch: String,
    pub channel: String,
    pub customer_id: Option<String>,
    pub coupon_code: Option<String>,
    pub now: DateTime<Utc>,
    pub max_spread_share: Option<Decimal>,
    pub segment_memberships: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuoteLineInput {
    pub currency: CurrencyCode,
    pub shop_side: Side,
    pub quantity: ForeignAmount,
    pub unit_size: UnitSize,
    pub base_rate: Rate,
    pub fee_thb: Money,
    pub published_buy_rate: Rate,
    pub published_sell_rate: Rate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedLine {
    pub base_rate: Rate,
    pub final_rate: Rate,
    pub fee_before: Money,
    pub fee_thb: Money,
    pub applied: Vec<AppliedBenefit>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub lines: Vec<EvaluatedLine>,
    pub snapshot: BenefitSnapshot,
}

pub struct BenefitEngine {
    pub active_rules: Arc<dyn ActiveRules>,
    pub registry: Arc<CalculatorRegistry>,
    pub tier_source: Arc<dyn TierSource>,
    pub consent_gate: Arc<dyn ConsentGate>,
    pub referral_source: Arc<dyn ReferralSource>,
}

impl BenefitEngine {
    pub fn new(
        active_rules: Arc<dyn ActiveRules>,
        registry: Arc<CalculatorRegistry>,
        tier_source: Arc<dyn TierSource>,
        consent_gate: Arc<dyn ConsentGate>,
        referral_source: Arc<dyn ReferralSource>,
    ) -> Self {
        Self {
            active_rules,
            registry,
            tier_source,
            consent_gate,
            referral_source,
        }
    }

    pub async fn evaluate(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ctx: &EvalContext,
        lines: &[QuoteLineInput],
    ) -> AppResult<Evaluation> {
        let active_rules = self
            .active_rules
            .active_for(tx, ctx.organization_id, &ctx.branch, ctx.now)
            .await?;

        let tier = ctx
            .customer_id
            .as_deref()
            .and_then(|cid| self.tier_source.tier_of(ctx.organization_id, cid));

        let marketing_consent = ctx
            .customer_id
            .as_deref()
            .map(|cid| self.consent_gate.marketing_allowed(cid))
            .unwrap_or(false);

        let is_referred = ctx
            .customer_id
            .as_deref()
            .map(|cid| self.referral_source.referred(cid, ctx.now))
            .unwrap_or(false);

        let mut evaluated_lines = Vec::with_capacity(lines.len());
        let mut all_applied = Vec::new();
        let mut skipped_rules = Vec::new();
        let mut seen_skipped = HashSet::new();
        let mut segment_snapshot_ids = Vec::new();
        let mut applied_rule_ids = HashSet::new();

        for line in lines {
            let line_thb = ExchangeArithmetic::thb(line.quantity, line.base_rate, line.unit_size);
            let thb_amount = Some(
                Decimal::try_from_i128_with_scale(line_thb.minor_i128(), 2)
                    .unwrap_or(Decimal::ZERO),
            );

            let mut eligible_rules = Vec::new();

            for active_rule in &active_rules {
                let in_segment = match active_rule.segment_snapshot_id {
                    Some(snap_id) => ctx.segment_memberships.contains(&snap_id),
                    None => false,
                };

                let elig_ctx = EligibilityContext {
                    branch: Some(&ctx.branch),
                    channel: Some(&ctx.channel),
                    customer_id: ctx.customer_id.as_deref(),
                    tier: tier.as_deref(),
                    currency: Some(line.currency.as_str()),
                    shop_side: Some(line.shop_side),
                    thb_amount,
                    in_segment,
                    marketing_consent,
                    is_referred,
                    has_coupon: ctx.coupon_code.is_some(),
                };

                match active_rule.rule.eligibility.check(&elig_ctx) {
                    Ok(()) => {
                        eligible_rules.push(active_rule.clone());
                    }
                    Err(reason) => {
                        if !seen_skipped.contains(&active_rule.rule.id) {
                            seen_skipped.insert(active_rule.rule.id);
                            skipped_rules.push(SkippedRule {
                                rule_id: active_rule.rule.id,
                                reason: reason.as_str().to_string(),
                            });
                        }
                    }
                }
            }

            let winners = Stacking::resolve(&eligible_rules);

            let mut current_final_rate = line.base_rate;
            let mut current_fee_thb = line.fee_thb;
            let mut line_applied = Vec::new();

            for &winning_rule in &winners {
                if let Some(calc) = self.registry.get(&winning_rule.rule.benefit_type) {
                    let effect = calc.apply(line, &winning_rule.rule.calculation);
                    let rf_result = RateFloor::apply(
                        line,
                        &effect,
                        winning_rule.rule.benefit_class,
                        ctx.max_spread_share,
                    );

                    current_final_rate = rf_result.final_rate;
                    current_fee_thb = rf_result.fee_thb;

                    let (rate_after, fee_after) = match winning_rule.rule.benefit_class {
                        crate::domain::benefit::rule::BenefitClass::Price => (
                            rf_result.final_rate.to_string(),
                            rf_result.fee_thb.to_string(),
                        ),
                        crate::domain::benefit::rule::BenefitClass::Reward => {
                            (line.base_rate.to_string(), line.fee_thb.to_string())
                        }
                    };

                    let applied = AppliedBenefit {
                        rule_id: winning_rule.rule.id,
                        version_id: winning_rule.version_id,
                        benefit_type: winning_rule.rule.benefit_type.clone(),
                        benefit_class: winning_rule.rule.benefit_class,
                        coupon_id: None,
                        reservation_id: None,
                        amount_thb: rf_result.amount_thb.to_string(),
                        rate_before: line.base_rate.to_string(),
                        rate_after,
                        fee_before: line.fee_thb.to_string(),
                        fee_after,
                        floor_capped: rf_result.floor_capped,
                    };

                    if let Some(snap_id) = winning_rule.segment_snapshot_id {
                        if !segment_snapshot_ids.contains(&snap_id) {
                            segment_snapshot_ids.push(snap_id);
                        }
                    }

                    applied_rule_ids.insert(winning_rule.rule.id);
                    all_applied.push(applied.clone());
                    line_applied.push(applied);
                }
            }

            evaluated_lines.push(EvaluatedLine {
                base_rate: line.base_rate,
                final_rate: current_final_rate,
                fee_before: line.fee_thb,
                fee_thb: current_fee_thb,
                applied: line_applied,
            });
        }

        // Only keep skipped rules that were never applied to any line
        skipped_rules.retain(|s| !applied_rule_ids.contains(&s.rule_id));

        let snapshot = BenefitSnapshot {
            engine_version: 1,
            applied: all_applied,
            skipped: skipped_rules,
            tier_code: tier,
            segment_snapshot_ids,
        };

        Ok(Evaluation {
            lines: evaluated_lines,
            snapshot,
        })
    }
}
