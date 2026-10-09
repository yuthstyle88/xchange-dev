//! Rate draft simulate command (§4.7, §6).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::domain::draft::rate_draft::DraftId;

use super::commands::{make_approval_service, map_rate_engine_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft_simulate.ts")]
pub struct SimulateDraftInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft_simulate.ts")]
pub struct SimulateDraftOutput {
    pub id: String,
    pub is_in_bound: bool,
    pub out_of_bound: Vec<String>,
}

pub struct SimulateDraftCommand {
    service: Arc<RateDraftService>,
}

impl SimulateDraftCommand {
    pub fn new(service: Arc<RateDraftService>) -> Self {
        Self { service }
    }
}

impl CommandAction for SimulateDraftCommand {
    const NAME: &'static str = "rateDraftSimulate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = SimulateDraftInput;
    type Output = SimulateDraftOutput;
    const ROUTE: &'static str = "/api/rate-drafts/{id}/simulate";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        _who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let draft_id = Uuid::from_str(&input.id).map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("id"),
                "invalid draft id",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let outcome = self
            .service
            .evaluate(tx, branch, &DraftId::new(draft_id))
            .await
            .map_err(map_rate_engine_error)?;

        Ok(SimulateDraftOutput {
            id: input.id,
            is_in_bound: outcome.evaluation.is_in_bound(),
            out_of_bound: outcome
                .evaluation
                .out_of_bound()
                .iter()
                .map(|(c, s)| format!("{}:{:?}", c.as_str(), s))
                .collect(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    let draft_service = Arc::new(RateDraftService::new(state.db.clone(), approval_service));
    CommandEndpoint::register(cfg, Arc::new(SimulateDraftCommand::new(draft_service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    SimulateDraftInput::export_all(&cfg)?;
    SimulateDraftOutput::export_all(&cfg)?;
    Ok(())
}
