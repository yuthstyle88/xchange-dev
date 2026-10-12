//! Rate draft publish command (§4.7, §6).

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
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::DraftId;

use super::commands::map_rate_engine_error;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::extractors::actor::legacy_actor;
use crate::presentation::http::v3::command::{role_not_allowed_here, CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft_publish.ts")]
pub struct PublishDraftInput {
    pub id: String,
    pub expected_version: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_draft_publish.ts")]
pub struct PublishDraftOutput {
    pub id: String,
    pub version: i32,
}

pub struct PublishDraftCommand {
    service: Arc<PublishService>,
}

impl PublishDraftCommand {
    pub fn new(service: Arc<PublishService>) -> Self {
        Self { service }
    }
}

impl CommandAction for PublishDraftCommand {
    const NAME: &'static str = "rateDraftPublish";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = PublishDraftInput;
    type Output = PublishDraftOutput;
    const ROUTE: &'static str = "/api/rate-drafts/{id}/publish";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
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

        // OR-7 bridge, removed by #1021
        let legacy = legacy_actor(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let pub_ref = self
            .service
            .publish(tx, branch, &legacy, &DraftId::new(draft_id), input.expected_version)
            .await
            .map_err(map_rate_engine_error)?;

        Ok(PublishDraftOutput {
            id: pub_ref.id.to_string(),
            version: pub_ref.version,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let publish_service = Arc::new(PublishService::new(state.db.clone()));
    CommandEndpoint::register(cfg, Arc::new(PublishDraftCommand::new(publish_service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PublishDraftInput::export_all(&cfg)?;
    PublishDraftOutput::export_all(&cfg)?;
    Ok(())
}
