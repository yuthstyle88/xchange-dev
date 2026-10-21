//! `plannerDraftSave` (APP-4 #994, design `customer-app.md` §4.3, §4.4): create a planner draft
//! (`POST /api/app/planner-drafts`) or update one at `expectedVersion` (`PUT …/{id}`).
//!
//! The owner is the signed-in customer, else the anonymous app session the seam issues and limits
//! (`AppSessionUse::IssueAndLimit`, #994 ruling option A).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use ts_rs::TS;
use uuid::Uuid;
use xc_app::application::planner::planner_service::PlannerService;
use xc_app::domain::app_error::CustomerAppError;
use xc_app::domain::owner::DraftOwner;
use xc_app::domain::planner::planner_draft::DraftSave;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::planner::{PlannerDraftDto, PlannerDraftSaveInput};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{
    AppSessionUse, CommandAction, CommandEndpoint, server_error,
};
use crate::presentation::http::v3::error::V3Error;

/// The v3 answer of a customer-app refusal (`CustomerAppError::code`).
pub(crate) fn customer_app_error(err: CustomerAppError) -> V3Error {
    let code = err.code();
    match err {
        CustomerAppError::AppSettingsMissing => {
            V3Error::new(code, None, "ยังไม่ได้ตั้งค่าอายุแผน", StatusCode::BAD_REQUEST)
        }
        CustomerAppError::StaleVersion => V3Error::new(
            code,
            None,
            "ข้อมูลถูกแก้ไขแล้ว กรุณาโหลดใหม่",
            StatusCode::CONFLICT,
        ),
        CustomerAppError::OutOfScope => {
            V3Error::new(code, None, "ไม่มีสิทธิ์ทำรายการนี้", StatusCode::FORBIDDEN)
        }
        CustomerAppError::NotFound => V3Error::new(code, None, "ไม่พบรายการ", StatusCode::NOT_FOUND),
        CustomerAppError::RateUnavailable(why) => {
            V3Error::new(code, None, why, StatusCode::CONFLICT)
        }
        CustomerAppError::ValidationFailed { field } => {
            V3Error::new(code, Some(field), "ข้อมูลไม่ถูกต้อง", StatusCode::BAD_REQUEST)
        }
        CustomerAppError::Server(why) => {
            tracing::error!(error = %why, "customer app service error");
            server_error()
        }
    }
}

/// The draft owner of a caller: the signed-in customer, else the app session; none → `signIn`.
pub(crate) fn draft_owner(caller: &Caller) -> Option<DraftOwner> {
    match caller.user.as_ref().and_then(|u| u.customer_id.clone()) {
        Some(customer) => Some(DraftOwner::Customer(customer)),
        None => caller
            .app_session
            .as_ref()
            .map(|s| DraftOwner::Session(s.hash.clone())),
    }
}

pub(crate) fn sign_in() -> V3Error {
    V3Error::from(xc_auth::domain::policy::value_objects::policy_error::PolicyError::SignIn)
}

/// The organisation of `branch` (an anonymous caller has no actor to carry one).
async fn organization_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> Result<Uuid, V3Error> {
    sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "planner draft organisation lookup failed");
            server_error()
        })
}

async fn save(
    service: &PlannerService,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    caller: &Caller,
    input: PlannerDraftSaveInput,
) -> Result<PlannerDraftDto, V3Error> {
    let owner = draft_owner(caller).ok_or_else(sign_in)?;
    let id = match input.id.as_deref() {
        None => None,
        Some(raw) => {
            Some(Uuid::parse_str(raw).map_err(|_| customer_app_error(CustomerAppError::NotFound))?)
        }
    };
    let save = DraftSave {
        id,
        expected_version: input.expected_version,
        plan: input.plan(branch)?,
    };
    let organization_id = organization_of(tx, branch).await?;
    let draft = service
        .save_in(tx, organization_id, &owner, save, chrono::Utc::now())
        .await
        .map_err(customer_app_error)?;
    Ok(PlannerDraftDto::of(&draft))
}

/// `POST /api/app/planner-drafts`: a new draft.
pub struct PlannerDraftCreateCommand {
    service: Arc<PlannerService>,
}

impl CommandAction for PlannerDraftCreateCommand {
    const NAME: &'static str = "plannerDraftSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "plannerDraftSave");
    type Input = PlannerDraftSaveInput;
    type Output = PlannerDraftDto;
    const ROUTE: &'static str = "/api/app/planner-drafts";
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
        branch: &BranchId,
        caller: &Caller,
        mut input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        input.id = None;
        save(&self.service, tx, branch, caller, input).await
    }
}

/// `PUT /api/app/planner-drafts/{id}`: an update at `expectedVersion`.
pub struct PlannerDraftUpdateCommand {
    service: Arc<PlannerService>,
}

impl CommandAction for PlannerDraftUpdateCommand {
    const NAME: &'static str = "plannerDraftSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "plannerDraftSave");
    type Input = PlannerDraftSaveInput;
    type Output = PlannerDraftDto;
    const ROUTE: &'static str = "/api/app/planner-drafts/{id}";
    const METHOD: Method = Method::PUT;
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
        branch: &BranchId,
        caller: &Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        if input.id.is_none() {
            return Err(customer_app_error(CustomerAppError::NotFound));
        }
        save(&self.service, tx, branch, caller, input).await
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let service = Arc::new(PlannerService::new(state.db.clone()));
    CommandEndpoint::register(
        cfg,
        Arc::new(PlannerDraftCreateCommand {
            service: service.clone(),
        }),
    );
    CommandEndpoint::register(cfg, Arc::new(PlannerDraftUpdateCommand { service }));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PlannerDraftSaveInput::export_all(&cfg)?;
    PlannerDraftDto::export_all(&cfg)?;
    Ok(())
}
