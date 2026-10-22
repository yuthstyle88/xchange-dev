//! `campaignCreate` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, Sign};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::marketing::branch_scope::CampaignBranchScope;
use crate::presentation::http::v3;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_create.ts", rename_all = "camelCase")]
pub struct CampaignCreateInput {
    pub name: String,
    pub starts_at: String,
    pub ends_at: String,
    pub budget_thb: String,
    pub stack_group: String,
    pub priority: i32,
    pub branches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_create.ts", rename_all = "camelCase")]
pub struct CampaignCreateOutput {
    pub id: String,
    pub name: String,
    pub status: String,
}

pub struct CampaignCreateCommand;

impl CommandAction for CampaignCreateCommand {
    const NAME: &'static str = "campaignCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignCreate");
    type Input = CampaignCreateInput;
    type Output = CampaignCreateOutput;
    const ROUTE: &'static str = "/api/campaigns";
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
        if input.name.trim().is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("name"),
                "name cannot be blank",
                StatusCode::BAD_REQUEST,
            ));
        }
        let user_id = resolve_user_id(tx, who, org_id).await;
        let starts_at = chrono::DateTime::parse_from_rfc3339(&input.starts_at)
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("startsAt"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?
            .with_timezone(&chrono::Utc);
        let ends_at = chrono::DateTime::parse_from_rfc3339(&input.ends_at)
            .map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("endsAt"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?
            .with_timezone(&chrono::Utc);
        let budget_thb = v3::input::decimal(
            "budgetThb",
            &input.budget_thb,
            Bound::numeric(20, 2, Sign::NonNegative),
        )?;

        if ends_at <= starts_at {
            return Err(V3Error::new(
                "validationFailed",
                Some("endsAt"),
                "endsAt must be after startsAt",
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

        let campaign_id = Uuid::new_v4();
        let version_id = Uuid::new_v4();
        let now = chrono::Utc::now();

        sqlx::query(
            "INSERT INTO campaigns (id, organization_id, name, current_version_id, created_by, created_at) \
             VALUES ($1, $2, $3, NULL, $4, $5)",
        )
        .bind(campaign_id)
        .bind(org_id)
        .bind(&input.name)
        .bind(user_id)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

        sqlx::query(
            "INSERT INTO campaign_versions ( \
                 id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
                 max_uses, stack_group, priority, segment_snapshot_id, version_hash, approval_id, \
                 simulated_at, edited_at, created_by, created_at \
             ) VALUES ($1, $2, $3, 1, 'DRAFT', $4, $5, $6, NULL, $7, $8, NULL, NULL, NULL, NULL, $9, $10, $9)",
        )
        .bind(version_id)
        .bind(org_id)
        .bind(campaign_id)
        .bind(starts_at)
        .bind(ends_at)
        .bind(budget_thb)
        .bind(&input.stack_group)
        .bind(input.priority)
        .bind(now)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

        for branch in &input.branches {
            sqlx::query(
                "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(version_id)
            .bind(branch)
            .execute(&mut **tx)
            .await
            .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;
        }

        sqlx::query(
            "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
        )
        .bind(version_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| V3Error::new("database", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

        Ok(CampaignCreateOutput {
            id: campaign_id.to_string(),
            name: input.name,
            status: "DRAFT".to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CampaignCreateCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignCreateInput::export_all(&cfg)?;
    CampaignCreateOutput::export_all(&cfg)?;
    Ok(())
}

async fn resolve_user_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    who: &Actor,
    org_id: Uuid,
) -> Uuid {
    if let Ok(u) = Uuid::parse_str(&who.user_id) {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
            .bind(u)
            .fetch_one(&mut **tx)
            .await
            .unwrap_or(false);
        if exists {
            return u;
        }
    }
    let fallback: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE organization_id = $1 LIMIT 1")
            .bind(org_id)
            .fetch_optional(&mut **tx)
            .await
            .unwrap_or(None);
    fallback.unwrap_or_else(|| Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
}
