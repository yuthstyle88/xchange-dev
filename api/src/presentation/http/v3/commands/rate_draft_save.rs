//! Rate draft create and edit commands (§4.7, §6).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::domain::draft::rate_draft::DraftId;

use super::commands::{make_approval_service, map_rate_engine_error, parse_draft_line};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_drafts_cmd.ts")]
pub struct DraftLineInput {
    pub currency: String,
    pub buy_mode: String,
    pub sell_mode: String,
    #[serde(default)]
    pub manual_buy: Option<String>,
    #[serde(default)]
    pub manual_sell: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_drafts_cmd.ts")]
pub struct CreateDraftInput {
    pub lines: Vec<DraftLineInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_drafts_cmd.ts")]
pub struct EditDraftInput {
    pub id: String,
    pub expected_version: i32,
    pub lines: Vec<DraftLineInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_drafts_cmd.ts")]
pub struct DraftActionOutput {
    pub id: String,
    pub version: i32,
    pub status: String,
}

pub struct CreateDraftCommand {
    service: Arc<RateDraftService>,
}

impl CreateDraftCommand {
    pub fn new(service: Arc<RateDraftService>) -> Self {
        Self { service }
    }
}

impl CommandAction for CreateDraftCommand {
    const NAME: &'static str = "rateDraftCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = CreateDraftInput;
    type Output = DraftActionOutput;
    const ROUTE: &'static str = "/api/rate-drafts";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let mut lines = Vec::new();
        for l in input.lines {
            let dl = parse_draft_line(
                &l.currency,
                &l.buy_mode,
                &l.sell_mode,
                l.manual_buy.as_deref(),
                l.manual_sell.as_deref(),
                l.reason,
            )?;
            lines.push(dl);
        }

        let draft = self
            .service
            .create(tx, branch, who, lines)
            .await
            .map_err(map_rate_engine_error)?;

        Ok(DraftActionOutput {
            id: draft.id.to_string(),
            version: draft.version,
            status: draft.status.as_str().to_string(),
        })
    }
}

pub struct EditDraftCommand {
    service: Arc<RateDraftService>,
}

impl EditDraftCommand {
    pub fn new(service: Arc<RateDraftService>) -> Self {
        Self { service }
    }
}

impl CommandAction for EditDraftCommand {
    const NAME: &'static str = "rateDraftEdit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = EditDraftInput;
    type Output = DraftActionOutput;
    const ROUTE: &'static str = "/api/rate-drafts/{id}";
    const METHOD: Method = Method::PUT;

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

        let mut lines = Vec::new();
        for l in input.lines {
            let dl = parse_draft_line(
                &l.currency,
                &l.buy_mode,
                &l.sell_mode,
                l.manual_buy.as_deref(),
                l.manual_sell.as_deref(),
                l.reason,
            )?;
            lines.push(dl);
        }

        let draft = self
            .service
            .edit(
                tx,
                branch,
                who,
                &DraftId::new(draft_id),
                input.expected_version,
                lines,
            )
            .await
            .map_err(map_rate_engine_error)?;

        Ok(DraftActionOutput {
            id: draft.id.to_string(),
            version: draft.version,
            status: draft.status.as_str().to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let approval_service = make_approval_service(state.db.clone());
    let draft_service = Arc::new(RateDraftService::new(state.db.clone(), approval_service));
    CommandEndpoint::register(
        cfg,
        Arc::new(CreateDraftCommand::new(draft_service.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(EditDraftCommand::new(draft_service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DraftLineInput::export_all(&cfg)?;
    CreateDraftInput::export_all(&cfg)?;
    EditDraftInput::export_all(&cfg)?;
    DraftActionOutput::export_all(&cfg)?;
    Ok(())
}
