//! `BenefitEstimator` (design `marketing-campaigns.md` §6.4, E-1…E-8): the customer's indicative benefit on a plan.
//!
//! Judged on APP, keeping only rules that DESK admits too (OD-1 B), evaluated at `now` with the current
//! publication, no coupon (OD-2 A) and a fee of 0. It never reserves: it holds no `BudgetLedger` (E-4) and runs
//! in the caller's read transaction, which the caller rolls back.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_app::application::planner::planner_preview::PlannerEstimate;
use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan};
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{ExchangeArithmetic, Money, Rate};
use xc_marketing::domain::benefit::rule::BenefitClass;
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_marketing::infrastructure::pg_version_labels::PgVersionLabels;

use super::benefit_pricing::{BenefitPricing, BenefitPricingInput, PricingLine};

/// Why no benefit is estimated (E-7). Skip reasons are never sent to the customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoBenefitReason {
    /// No ACTIVE rule survives the budget and coupon filters: nothing runs for this branch now (row 5).
    NoActiveRule,
    /// Rules survive those filters; none applies to this caller and line. A rule that `also_admitted_on`
    /// drops (an APP-only rule under OD-1 B, row 4) counts here: it runs, but not for this channel.
    NotEligible,
}

/// One applied benefit of the estimate.
#[derive(Debug, Clone, PartialEq)]
pub struct EstimateItem {
    pub rule_id: Uuid,
    /// The campaign's `name`, as `my_benefits` (§6.3).
    pub label: String,
    pub benefit_type: String,
    pub benefit_class: BenefitClass,
    /// The applied entry's `amountThb`, `rateBefore`, `rateAfter`, `floorCapped`, verbatim.
    pub amount_thb: String,
    pub rate_before: String,
    pub rate_after: String,
    pub floor_capped: bool,
    /// The campaign version's `ends_at` (E-6).
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenefitEstimate {
    pub evaluated_at: DateTime<Utc>,
    /// The earliest `ends_at` of the items (E-6).
    pub valid_until: DateTime<Utc>,
    pub publication_id: Uuid,
    pub final_rate: Rate,
    /// `ExchangeArithmetic::try_thb(quantity, final_rate, unit_size)`, as the quote.
    pub thb_after: Money,
    /// Σ `amount_thb` of `items`.
    pub benefit_thb: Money,
    pub items: Vec<EstimateItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EstimateOutcome {
    Estimated(BenefitEstimate),
    None(NoBenefitReason),
}

pub struct BenefitEstimator {
    pricing: BenefitPricing,
    labels: PgVersionLabels,
}

impl BenefitEstimator {
    pub fn new(campaign_repo: Arc<PgCampaignRepository>) -> Self {
        Self {
            pricing: BenefitPricing::new(campaign_repo),
            labels: PgVersionLabels::new(),
        }
    }

    /// The estimate of `plan` priced at `planner.published`, for `customer_id` (already narrowed by the caller
    /// to a customer of `organization_id`, else `None`, E-5). Reads only.
    #[allow(clippy::too_many_arguments)]
    pub async fn estimate_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        branch: &BranchId,
        customer_id: Option<&str>,
        planner: &PlannerEstimate,
        plan: &DraftPlan,
        now: DateTime<Utc>,
    ) -> AppResult<EstimateOutcome> {
        let quantity = match plan.amount {
            DraftAmount::Quantity(q) => q,
            DraftAmount::BudgetThb(_) => {
                return Err(AppError::Internal(
                    "BudgetThb cannot reach BenefitEstimator::estimate_in".to_string(),
                ));
            }
        };

        let shop_side = plan.customer_side.to_shop();

        let priced = self
            .pricing
            .evaluate_in(
                tx,
                BenefitPricingInput {
                    organization_id,
                    branch,
                    eligibility_channel: QuoteChannel::App,
                    also_admitted_on: Some(QuoteChannel::Desk),
                    customer_id,
                    coupon: None,
                    coupon_code: None,
                    line: PricingLine {
                        currency: plan.currency,
                        shop_side,
                        quantity,
                        fee_thb: Money::ZERO,
                        published: planner.published.clone(),
                    },
                    now,
                },
            )
            .await?;

        let applied = priced.evaluation.snapshot.applied;
        if applied.is_empty() {
            let reason = if priced.active_rules.len() + priced.dropped_by_also_admitted_on == 0 {
                NoBenefitReason::NoActiveRule
            } else {
                NoBenefitReason::NotEligible
            };
            return Ok(EstimateOutcome::None(reason));
        }

        let version_ids: Vec<Uuid> = applied.iter().map(|e| e.version_id).collect();
        let labels = self
            .labels
            .labels_in(tx, organization_id, &version_ids)
            .await?;

        let mut benefit_minor: i128 = 0;
        let mut items = Vec::with_capacity(applied.len());
        for entry in applied {
            let label = labels.get(&entry.version_id).ok_or_else(|| {
                AppError::Internal(format!(
                    "missing campaign version label for version {}",
                    entry.version_id
                ))
            })?;
            let item_money = Money::parse(&entry.amount_thb).map_err(|_| {
                AppError::Internal(format!(
                    "applied benefit {} has an unreadable amountThb {:?}",
                    entry.rule_id, entry.amount_thb
                ))
            })?;
            benefit_minor = benefit_minor
                .checked_add(item_money.minor_i128())
                .ok_or_else(|| {
                    AppError::Internal(format!("benefit_thb overflow at rule {}", entry.rule_id))
                })?;
            items.push(EstimateItem {
                rule_id: entry.rule_id,
                label: label.name.clone(),
                benefit_type: entry.benefit_type,
                benefit_class: entry.benefit_class,
                amount_thb: entry.amount_thb,
                rate_before: entry.rate_before,
                rate_after: entry.rate_after,
                floor_capped: entry.floor_capped,
                ends_at: label.ends_at,
            });
        }

        let benefit_thb = Money::try_from_minor_i128(benefit_minor)
            .map_err(|_| AppError::Internal("benefit_thb exceeds Money capacity".to_string()))?;
        let valid_until = items
            .iter()
            .map(|i| i.ends_at)
            .min()
            .expect("applied is non-empty");
        let publication_id = planner.published.publication.id.as_uuid();
        let final_rate = priced.evaluation.lines[0].final_rate;
        let thb_after =
            ExchangeArithmetic::try_thb(quantity, final_rate, planner.published.unit_size)
                .map_err(|_| {
                    AppError::Domain("validationFailed: quantity overflows thb amount".to_string())
                })?;

        Ok(EstimateOutcome::Estimated(BenefitEstimate {
            evaluated_at: now,
            valid_until,
            publication_id,
            final_rate,
            thb_after,
            benefit_thb,
            items,
        }))
    }
}
