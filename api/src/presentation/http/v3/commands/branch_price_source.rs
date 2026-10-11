//! Branch price source command (§3.5, §6).
//!
//! Release gate (design doc §6):
//! No branch is set to 'RATE_BOARD' until F6, F7 and F8 are merged and deployed.
//!
//! Switching back (owner ruling, 2026-10-10):
//! A switch from 'RATE_BOARD' to 'BOT_DRAFT' is refused unless an approved BoT draft
//! is published in the same command, in the same transaction. The command takes an optional
//! `draftId` for this.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::price_source::{BranchPriceSource, PriceSource};
use xc_rate::domain::draft::rate_draft::DraftId;
use xc_rate::infrastructure::branch::price_source::PgBranchPriceSource;

use super::commands::map_rate_engine_error;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::extractors::actor::legacy_actor;
use crate::presentation::http::v3::command::{role_not_allowed_here, CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_price_source.ts")]
pub struct BranchPriceSourceInput {
    #[ts(type = "\"BOT_DRAFT\" | \"RATE_BOARD\"")]
    pub price_source: PriceSource,
    pub draft_id: Option<String>,
    pub expected_version: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_price_source.ts")]
pub struct BranchPriceSourceOutput {
    pub branch_id: String,
    #[ts(type = "\"BOT_DRAFT\" | \"RATE_BOARD\"")]
    pub price_source: PriceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
}

/// The command `branchPriceSourceSet`.
///
/// Release gate: No branch is set to 'RATE_BOARD' until F6, F7 and F8 are merged and deployed.
pub struct BranchPriceSourceCommand {
    sources: Arc<dyn BranchPriceSource>,
}

impl BranchPriceSourceCommand {
    pub fn new(sources: Arc<dyn BranchPriceSource>) -> Self {
        Self { sources }
    }
}

impl CommandAction for BranchPriceSourceCommand {
    const NAME: &'static str = "branchPriceSourceSet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = BranchPriceSourceInput;
    type Output = BranchPriceSourceOutput;
    const ROUTE: &'static str = "/api/branches/price-source";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // OR-7 bridge, removed by #1026
        let legacy = legacy_actor(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        match (input.price_source, input.draft_id) {
            (PriceSource::BotDraft, Some(draft_id_str)) => {
                let draft_uuid = Uuid::from_str(&draft_id_str).map_err(|_| {
                    V3Error::new(
                        "validationFailed",
                        Some("draftId"),
                        "invalid draft id",
                        StatusCode::BAD_REQUEST,
                    )
                })?;
                let draft_id = DraftId::new(draft_uuid);
                let pub_ref = self
                    .sources
                    .switch_back(
                        tx,
                        &legacy,
                        branch,
                        &draft_id,
                        input.expected_version,
                    )
                    .await
                    .map_err(map_rate_engine_error)?;

                Ok(BranchPriceSourceOutput {
                    branch_id: branch.as_str().to_string(),
                    price_source: PriceSource::BotDraft,
                    publication_id: Some(pub_ref.id.to_string()),
                    version: Some(pub_ref.version),
                })
            }
            (PriceSource::BotDraft, None) => {
                self.sources
                    .set(tx, &legacy, branch, PriceSource::BotDraft)
                    .await
                    .map_err(map_rate_engine_error)?;

                Ok(BranchPriceSourceOutput {
                    branch_id: branch.as_str().to_string(),
                    price_source: PriceSource::BotDraft,
                    publication_id: None,
                    version: None,
                })
            }
            (PriceSource::RateBoard, _) => {
                self.sources
                    .set(tx, &legacy, branch, PriceSource::RateBoard)
                    .await
                    .map_err(map_rate_engine_error)?;

                Ok(BranchPriceSourceOutput {
                    branch_id: branch.as_str().to_string(),
                    price_source: PriceSource::RateBoard,
                    publication_id: None,
                    version: None,
                })
            }
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let audit_log: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(state.db.clone()));
    let sources = Arc::new(PgBranchPriceSource::new(state.db.clone(), audit_log));
    CommandEndpoint::register(cfg, Arc::new(BranchPriceSourceCommand::new(sources)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchPriceSourceInput::export_all(&cfg)?;
    BranchPriceSourceOutput::export_all(&cfg)?;
    Ok(())
}
