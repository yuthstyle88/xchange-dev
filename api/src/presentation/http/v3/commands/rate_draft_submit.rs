//! Rate draft submit command (§4.7, §6).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::domain::draft::rate_draft::DraftId;

use super::commands::{make_approval_service, map_rate_engine_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::extractors::actor::legacy_actor;
use crate::presentation::http::v3::command::{role_not_allowed_here, CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft_submit.ts")]
pub struct SubmitDraftInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft_submit.ts")]
pub struct SubmitDraftOutput {
    pub request_id: String,
    pub status: String,
}

pub struct SubmitDraftCommand {
    service: Arc<RateDraftService>,
}

impl SubmitDraftCommand {
    pub fn new(service: Arc<RateDraftService>) -> Self {
        Self { service }
    }
}

impl CommandAction for SubmitDraftCommand {
    const NAME: &'static str = "rateDraftSubmit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = SubmitDraftInput;
    type Output = SubmitDraftOutput;
    const ROUTE: &'static str = "/api/rate-drafts/{id}/submit";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let draft_id = Uuid::from_str(&input.id).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("id"),
                "invalid draft id",
                StatusCode::BAD_REQUEST,
            )
        })?;

        // OR-7 bridge, removed by #1021
        let legacy = legacy_actor(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let _ = &legacy;
        let outcome = self
            .service
            .submit(tx, branch, who, &DraftId::new(draft_id), key.as_str())
            .await
            .map_err(map_rate_engine_error)?;

        Ok(SubmitDraftOutput {
            request_id: outcome.request_id.to_string(),
            status: outcome.status.as_str().to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    let draft_service = Arc::new(RateDraftService::new(state.db.clone(), approval_service));
    CommandEndpoint::register(cfg, Arc::new(SubmitDraftCommand::new(draft_service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    SubmitDraftInput::export_all(&cfg)?;
    SubmitDraftOutput::export_all(&cfg)?;
    Ok(())
}
