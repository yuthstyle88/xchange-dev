//! `plannerDraftDelete` (APP-4 #994, design `customer-app.md` §4.3, §4.4):
//! `POST /api/app/planner-drafts/{id}/delete`. Another owner's draft → `outOfScope`, kept.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use ts_rs::TS;
use uuid::Uuid;
use xc_app::application::planner::planner_service::PlannerService;
use xc_app::domain::app_error::CustomerAppError;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::planner::{PlannerDraftDeleteInput, PlannerDraftDeleteOutput};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{AppSessionUse, CommandAction, CommandEndpoint};
use crate::presentation::http::v3::planner_draft_save::{
    customer_app_error, draft_owner, sign_in,
};
use crate::presentation::http::v3::error::V3Error;

pub struct PlannerDraftDeleteCommand {
    service: PlannerService,
}

impl CommandAction for PlannerDraftDeleteCommand {
    const NAME: &'static str = "plannerDraftDelete";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "plannerDraftDelete");
    type Input = PlannerDraftDeleteInput;
    type Output = PlannerDraftDeleteOutput;
    const ROUTE: &'static str = "/api/app/planner-drafts/{id}/delete";
    const METHOD: Method = Method::POST;
    const APP_SESSION: AppSessionUse = AppSessionUse::IssueAndLimit;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // Served through `run_as`; the seam never calls `run` for this command.
        Err(sign_in())
    }

    async fn run_as(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        caller: &Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let owner = draft_owner(caller).ok_or_else(sign_in)?;
        let id = Uuid::parse_str(&input.id)
            .map_err(|_| customer_app_error(CustomerAppError::NotFound))?;
        self.service
            .delete_in(tx, &owner, id)
            .await
            .map_err(customer_app_error)?;
        Ok(PlannerDraftDeleteOutput { id: input.id })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(PlannerDraftDeleteCommand {
            service: PlannerService::new(state.db.clone()),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PlannerDraftDeleteInput::export_all(&cfg)?;
    PlannerDraftDeleteOutput::export_all(&cfg)?;
    Ok(())
}
