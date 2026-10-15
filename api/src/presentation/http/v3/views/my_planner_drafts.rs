//! The view `my_planner_drafts` (APP-4 #994, design `customer-app.md` §4.4): the caller's planner
//! drafts, newest first. The owner is the signed-in customer, else the `xc_app_sid` session the
//! request carries; a caller with neither has no drafts.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use ts_rs::TS;
use pos_error::{AppError, AppResult};
use xc_app::application::planner::planner_service::PlannerService;
use xc_app::domain::app_error::CustomerAppError;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::planner::{
    MyPlannerDraftsData, MyPlannerDraftsFilter, PlannerDraftDto,
};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::planner_draft_save::draft_owner;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct MyPlannerDraftsView {
    service: Arc<PlannerService>,
}

impl BranchView for MyPlannerDraftsView {
    const NAME: &'static str = "my_planner_drafts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_planner_drafts");
    /// Drafts are not branch data: the branch's version never says whether they changed.
    const VERSIONED: bool = false;
    type Data = MyPlannerDraftsData;
    type Filter = MyPlannerDraftsFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let caller = Caller {
            user: Some(who.clone()),
            app_session: None,
            session_hash: None,
        };
        let branch = branch.clone();
        let filter = filter.clone();
        async move { self.load_as(&branch, &caller, &filter).await }
    }

    fn load_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let owner = draft_owner(caller);
        let service = self.service.clone();
        async move {
            let drafts = match owner {
                None => Vec::new(),
                Some(owner) => service
                    .list(&owner)
                    .await
                    .map_err(|e| {
                        // `CustomerAppError` has no `Display`; log its message or code, never `Debug`.
                        let why = match e {
                            CustomerAppError::Server(why) => why,
                            other => other.code().to_owned(),
                        };
                        AppError::Internal(format!("my_planner_drafts: {why}"))
                    })?,
            };
            Ok(MyPlannerDraftsData {
                drafts: drafts.iter().map(PlannerDraftDto::of).collect(),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(MyPlannerDraftsView {
            service: Arc::new(PlannerService::new(state.db.clone())),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyPlannerDraftsData::export_all(&cfg)?;
    MyPlannerDraftsFilter::export_all(&cfg)?;
    Ok(())
}
