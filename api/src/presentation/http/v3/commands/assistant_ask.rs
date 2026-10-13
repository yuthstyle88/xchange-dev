//! `assistantAsk` (APP-6 #997, design `customer-app.md` §4.3, §4.4): ask the XCHANGE Assistant
//! (`POST /api/app/assistant/messages`).
//!
//! The owner is the signed-in customer, else the anonymous app session the seam issues and limits
//! (`AppSessionUse::IssueAndLimit`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use ts_rs::TS;
use uuid::Uuid;
use xc_app::application::assistant::assistant_service::AssistantService;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::assistant::{AssistantAskAnswerDto, AssistantAskInput};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{
    AppSessionUse, CommandAction, CommandEndpoint, server_error,
};
use crate::presentation::http::v3::planner_draft_save::{customer_app_error, draft_owner, sign_in};
use crate::presentation::http::v3::error::V3Error;

async fn organization_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> Result<Uuid, V3Error> {
    sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "assistant branch organisation lookup failed");
            server_error()
        })
}

pub struct AssistantAskCommand {
    service: Arc<AssistantService>,
}

impl CommandAction for AssistantAskCommand {
    const NAME: &'static str = "assistantAsk";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "assistantAsk");
    type Input = AssistantAskInput;
    type Output = AssistantAskAnswerDto;
    const ROUTE: &'static str = "/api/app/assistant/messages";
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
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let owner = draft_owner(caller).ok_or_else(sign_in)?;
        let text = input.text.trim();
        if text.is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("text"),
                "ข้อความต้องไม่ว่าง",
                StatusCode::BAD_REQUEST,
            ));
        }

        let locale = match input.locale.as_deref() {
            Some("th") => "th",
            Some("zh") => "zh",
            Some("en") => "en",
            _ => "th",
        };

        let organization_id = organization_of(tx, branch).await?;
        let now = chrono::Utc::now();
        let idem_str = key.as_str();
        let idem_opt = if idem_str.is_empty() {
            None
        } else {
            Some(idem_str)
        };

        let answer = self
            .service
            .ask_in(tx, organization_id, &owner, locale, text, idem_opt, now)
            .await
            .map_err(customer_app_error)?;

        Ok(AssistantAskAnswerDto::of(&answer))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(AssistantAskCommand {
            service: Arc::new(AssistantService::new(state.db.clone())),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AssistantAskInput::export_all(&cfg)?;
    AssistantAskAnswerDto::export_all(&cfg)?;
    Ok(())
}
