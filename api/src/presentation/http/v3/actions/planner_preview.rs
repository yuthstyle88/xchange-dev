//! The compute `planner_preview` (APP-4 #994, design `customer-app.md` P5, §4.3, §4.4): a fresh
//! estimate of a plan at the endpoint's branch. It writes nothing: no quote, no stock, no booking.
//! Public on the route table, so an anonymous visitor reaches it.

use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_app::application::planner::planner_preview::PlannerPreview;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;

use crate::application::workflows::exchange::benefit_estimate::{
    BenefitEstimator, EstimateOutcome, NoBenefitReason,
};
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::planner::{
    PlannerBenefitDto, PlannerBenefitItemDto, PlannerPreviewAnswer, PlannerPreviewRequest,
};
use crate::presentation::http::v3::action::{ComputeAction, ComputeEndpoint, server_error};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::planner_draft_save::customer_app_error;

pub struct PlannerPreviewAction {
    pool: PgPool,
    preview: PlannerPreview,
    estimator: BenefitEstimator,
}

impl PlannerPreviewAction {
    async fn estimate(
        &self,
        branch: &BranchId,
        request: PlannerPreviewRequest,
        customer: Option<&Actor>,
    ) -> Result<PlannerPreviewAnswer, V3Error> {
        let plan = request.plan(branch)?;
        let now = chrono::Utc::now();
        let mut tx = self.pool.begin().await.map_err(|e| {
            tracing::error!(error = %e, "planner preview transaction failed");
            server_error()
        })?;
        let result = match self.preview.estimate(&mut tx, &plan, now).await {
            Ok(estimate) => {
                match sqlx::query_scalar::<_, Uuid>(
                    "SELECT organization_id FROM branches WHERE id = $1",
                )
                .bind(branch.as_str())
                .fetch_one(&mut *tx)
                .await
                {
                    Ok(org) => {
                        let customer_id = customer
                            .filter(|a| Uuid::parse_str(&a.organization_id).ok() == Some(org))
                            .and_then(|a| a.customer_id.as_deref());
                        self.estimator
                            .estimate_in(&mut tx, org, branch, customer_id, &estimate, &plan, now)
                            .await
                            .map(|outcome| (estimate, outcome))
                            .map_err(|e| {
                                tracing::error!(error = %e, "planner preview benefit estimate failed");
                                server_error()
                            })
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "planner preview branch organisation read failed");
                        Err(server_error())
                    }
                }
            }
            Err(e) => Err(customer_app_error(e)),
        };
        // The preview reads only; nothing in this transaction is kept.
        if let Err(e) = tx.rollback().await {
            tracing::error!(error = %e, "planner preview rollback failed");
        }
        let (estimate, outcome) = result?;
        let benefit = match outcome {
            EstimateOutcome::Estimated(e) => PlannerBenefitDto::Estimated {
                basis: "INDICATIVE".to_owned(),
                channel: "APP".to_owned(),
                evaluated_at: e.evaluated_at.to_rfc3339(),
                valid_until: e.valid_until.to_rfc3339(),
                publication_id: e.publication_id.to_string(),
                final_rate: e.final_rate.to_string(),
                thb_after: e.thb_after.to_string(),
                benefit_thb: e.benefit_thb.to_string(),
                items: e
                    .items
                    .into_iter()
                    .map(|i| PlannerBenefitItemDto {
                        rule_id: i.rule_id.to_string(),
                        label: i.label,
                        benefit_type: i.benefit_type,
                        benefit_class: i.benefit_class.as_str().to_owned(),
                        amount_thb: i.amount_thb,
                        rate_before: i.rate_before,
                        rate_after: i.rate_after,
                        floor_capped: i.floor_capped,
                        ends_at: i.ends_at.to_rfc3339(),
                    })
                    .collect(),
            },
            EstimateOutcome::None(reason) => PlannerBenefitDto::None {
                basis: "INDICATIVE".to_owned(),
                channel: "APP".to_owned(),
                evaluated_at: now.to_rfc3339(),
                reason: match reason {
                    NoBenefitReason::NoActiveRule => "noActiveRule",
                    NoBenefitReason::NotEligible => "notEligible",
                }
                .to_owned(),
            },
        };
        Ok(PlannerPreviewAnswer {
            thb: estimate.thb.to_string(),
            rate: estimate.rate.to_string(),
            unit_size: estimate.unit_size.to_string(),
            stale: estimate.stale,
            source_date: estimate.source_date.map(|d| d.to_string()),
            benefit,
        })
    }
}

impl ComputeAction for PlannerPreviewAction {
    const NAME: &'static str = "planner_preview";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "planner_preview");
    type Request = PlannerPreviewRequest;
    type Answer = PlannerPreviewAnswer;

    async fn compute(
        &self,
        branch: &BranchId,
        _who: &Actor,
        request: Self::Request,
    ) -> Result<Self::Answer, V3Error> {
        self.estimate(branch, request, None).await
    }

    async fn compute_as(
        &self,
        branch: &BranchId,
        caller: &Caller,
        request: Self::Request,
    ) -> Result<Self::Answer, V3Error> {
        self.estimate(branch, request, caller.user.as_ref()).await
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ComputeEndpoint::register(
        cfg,
        Arc::new(PlannerPreviewAction {
            pool: state.db.clone(),
            preview: PlannerPreview::new(state.db.clone()),
            estimator: BenefitEstimator::new(Arc::new(PgCampaignRepository::new())),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PlannerPreviewRequest::export_all(&cfg)?;
    PlannerPreviewAnswer::export_all(&cfg)?;
    PlannerBenefitDto::export_all(&cfg)?;
    PlannerBenefitItemDto::export_all(&cfg)?;
    Ok(())
}
