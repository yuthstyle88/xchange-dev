//! The compute `planner_preview` (APP-4 #994, design `customer-app.md` P5, §4.3, §4.4): a fresh
//! estimate of a plan at the endpoint's branch. It writes nothing: no quote, no stock, no booking.
//! Public on the route table, so an anonymous visitor reaches it.

use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use sqlx::PgPool;
use ts_rs::TS;
use xc_app::application::planner::planner_preview::PlannerPreview;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::planner::{PlannerPreviewAnswer, PlannerPreviewRequest};
use crate::presentation::http::v3::action::{ComputeAction, ComputeEndpoint, server_error};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::planner_draft_save::customer_app_error;

pub struct PlannerPreviewAction {
    pool: PgPool,
    preview: PlannerPreview,
}

impl PlannerPreviewAction {
    async fn estimate(
        &self,
        branch: &BranchId,
        request: PlannerPreviewRequest,
    ) -> Result<PlannerPreviewAnswer, V3Error> {
        let plan = request.plan(branch)?;
        let mut tx = self.pool.begin().await.map_err(|e| {
            tracing::error!(error = %e, "planner preview transaction failed");
            server_error()
        })?;
        let estimate = self
            .preview
            .estimate(&mut tx, &plan, chrono::Utc::now())
            .await;
        // The preview reads only; nothing in this transaction is kept.
        if let Err(e) = tx.rollback().await {
            tracing::error!(error = %e, "planner preview rollback failed");
        }
        let estimate = estimate.map_err(customer_app_error)?;
        Ok(PlannerPreviewAnswer {
            thb: estimate.thb.to_string(),
            rate: estimate.rate.to_string(),
            unit_size: estimate.unit_size.to_string(),
            stale: estimate.stale,
            source_date: estimate.source_date.map(|d| d.to_string()),
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
        self.estimate(branch, request).await
    }

    async fn compute_as(
        &self,
        branch: &BranchId,
        _caller: &Caller,
        request: Self::Request,
    ) -> Result<Self::Answer, V3Error> {
        self.estimate(branch, request).await
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ComputeEndpoint::register(
        cfg,
        Arc::new(PlannerPreviewAction {
            pool: state.db.clone(),
            preview: PlannerPreview::new(state.db.clone()),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PlannerPreviewRequest::export_all(&cfg)?;
    PlannerPreviewAnswer::export_all(&cfg)?;
    Ok(())
}
