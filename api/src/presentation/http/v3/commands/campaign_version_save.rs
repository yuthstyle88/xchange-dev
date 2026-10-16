//! `campaignVersionSave` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::marketing::branch_scope::CampaignBranchScope;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_version_save.ts", rename_all = "camelCase")]
pub struct CampaignVersionSaveInput {
    pub campaign_id: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub id: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub budget_thb: String,
    pub stack_group: String,
    pub priority: i32,
    pub branches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_version_save.ts", rename_all = "camelCase")]
pub struct CampaignVersionSaveOutput {
    pub version_id: String,
    pub version: i32,
    pub status: String,
}

pub struct CampaignVersionSaveCommand;

impl CommandAction for CampaignVersionSaveCommand {
    const NAME: &'static str = "campaignVersionSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignVersionSave");
    type Input = CampaignVersionSaveInput;
    type Output = CampaignVersionSaveOutput;
    const ROUTE: &'static str = "/api/campaigns/{id}/draft";
    const METHOD: Method = Method::PUT;

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

        let starts_at = chrono::DateTime::parse_from_rfc3339(&input.starts_at)
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("startsAt"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?
            .with_timezone(&Utc);
        let ends_at = chrono::DateTime::parse_from_rfc3339(&input.ends_at)
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("endsAt"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?
            .with_timezone(&Utc);
        let budget_thb = Decimal::from_str(&input.budget_thb).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("budgetThb"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        if ends_at <= starts_at {
            return Err(V3Error::new(
                "validationFailed",
                Some("endsAt"),
                "endsAt must be after startsAt",
                StatusCode::BAD_REQUEST,
            ));
        }
        if budget_thb < Decimal::ZERO {
            return Err(V3Error::new(
                "validationFailed",
                Some("budgetThb"),
                "budgetThb must not be negative",
                StatusCode::BAD_REQUEST,
            ));
        }
        if CampaignBranchScope::first_outside_org_in(tx, org_id, &input.branches)
            .await
            .map_err(|e| {
                V3Error::new(
                    "database",
                    None,
                    e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?
            .is_some()
        {
            return Err(V3Error::new(
                "validationFailed",
                Some("branches"),
                "branch does not belong to organization",
                StatusCode::BAD_REQUEST,
            ));
        }

        let row = sqlx::query(
            "SELECT id, version FROM campaign_versions \
             WHERE organization_id = $1 AND campaign_id = $2 AND status IN ('DRAFT', 'SIMULATED') \
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
        })?;

        let Some(r) = row else {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                     SELECT 1 FROM campaign_versions \
                     WHERE organization_id = $1 AND campaign_id = $2 \
                 )",
            )
            .bind(org_id)
            .bind(campaign_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| {
                V3Error::new(
                    "database",
                    None,
                    e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?;
            if exists {
                return Err(V3Error::new(
                    "staleVersion",
                    None,
                    "campaign version is not editable; create a new draft version instead",
                    StatusCode::CONFLICT,
                ));
            }
            return Err(V3Error::new(
                "versionNotFound",
                None,
                "no draft version found for campaign",
                StatusCode::NOT_FOUND,
            ));
        };

        let version_id: Uuid = r.get("id");
        let version_num: i32 = r.get("version");
        let now = Utc::now();

        sqlx::query(
            "UPDATE campaign_versions SET \
                 status = 'DRAFT', \
                 starts_at = $1, \
                 ends_at = $2, \
                 budget_thb = $3, \
                 stack_group = $4, \
                 priority = $5, \
                 edited_at = $6, \
                 version_hash = NULL \
             WHERE id = $7",
        )
        .bind(starts_at)
        .bind(ends_at)
        .bind(budget_thb)
        .bind(&input.stack_group)
        .bind(input.priority)
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

        sqlx::query("DELETE FROM campaign_branches WHERE campaign_version_id = $1")
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

        for branch in &input.branches {
            sqlx::query(
                "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(version_id)
            .bind(branch)
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
        }

        Ok(CampaignVersionSaveOutput {
            version_id: version_id.to_string(),
            version: version_num,
            status: "DRAFT".to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CampaignVersionSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignVersionSaveInput::export_all(&cfg)?;
    CampaignVersionSaveOutput::export_all(&cfg)?;
    Ok(())
}
