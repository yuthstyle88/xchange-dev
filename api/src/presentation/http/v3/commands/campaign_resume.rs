//! `campaignResume` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
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

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_resume.ts", rename_all = "camelCase")]
pub struct CampaignResumeInput {
    pub campaign_id: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_resume.ts", rename_all = "camelCase")]
pub struct CampaignResumeOutput {
    pub campaign_id: String,
    pub status: String,
}

pub struct CampaignResumeCommand;

impl CommandAction for CampaignResumeCommand {
    const NAME: &'static str = "campaignResume";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignResume");
    type Input = CampaignResumeInput;
    type Output = CampaignResumeOutput;
    const ROUTE: &'static str = "/api/campaigns/{id}/resume";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org_id = Uuid::parse_str(&who.organization_id).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("organizationId"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let campaign_id_str = input
            .campaign_id
            .as_deref()
            .or(input.id.as_deref())
            .ok_or_else(|| {
                V3Error::new(
                    "validationFailed",
                    Some("id"),
                    "campaign id is required",
                    StatusCode::BAD_REQUEST,
                )
            })?;
        let campaign_id = Uuid::parse_str(campaign_id_str).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("id"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        let version_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM campaign_versions \
             WHERE organization_id = $1 AND campaign_id = $2 AND status = 'PAUSED' \
             ORDER BY version DESC LIMIT 1 FOR UPDATE",
        )
        .bind(org_id)
        .bind(campaign_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| {
            V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?
        .ok_or_else(|| {
            V3Error::new(
                "versionNotFound",
                None,
                "no paused version found for campaign",
                StatusCode::NOT_FOUND,
            )
        })?;

        let now = Utc::now();
        sqlx::query(
            "UPDATE campaign_versions SET status = 'APPROVED', edited_at = $1 WHERE id = $2",
        )
        .bind(now)
        .bind(version_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        Ok(CampaignResumeOutput {
            campaign_id: campaign_id.to_string(),
            status: "APPROVED".to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CampaignResumeCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignResumeInput::export_all(&cfg)?;
    CampaignResumeOutput::export_all(&cfg)?;
    Ok(())
}
