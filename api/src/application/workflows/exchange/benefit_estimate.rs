//! `BenefitEstimator` (design `marketing-campaigns.md` §6.4, E-1…E-8): the customer's indicative benefit on a plan.
//!
//! Judged on APP, keeping only rules that DESK admits too (OD-1 B), evaluated at `now` with the current
//! publication, no coupon (OD-2 A) and a fee of 0. It never reserves: it holds no `BudgetLedger` (E-4) and runs
//! in the caller's read transaction, which the caller rolls back.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_app::application::planner::planner_preview::PlannerEstimate;
use xc_app::domain::planner::planner_draft::DraftPlan;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Money, Rate};
use xc_marketing::domain::benefit::rule::BenefitClass;
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_marketing::infrastructure::pg_version_labels::PgVersionLabels;

use super::benefit_pricing::BenefitPricing;

/// Why no benefit is estimated (E-7). Skip reasons are never sent to the customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoBenefitReason {
    /// No ACTIVE rule survives the budget, coupon and channel filters.
    NoActiveRule,
    /// Rules exist; none applies to this caller and line.
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
        let _ = (
            &self.pricing,
            &self.labels,
            tx,
            organization_id,
            branch,
            customer_id,
            planner,
            plan,
            now,
        );
        todo!("BE-3: BenefitEstimator::estimate_in")
    }
}
