//! `campaignVersionNew` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
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
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_version_new.ts", rename_all = "camelCase")]
pub struct CampaignVersionNewInput {
    pub campaign_id: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_version_new.ts", rename_all = "camelCase")]
pub struct CampaignVersionNewOutput {
    pub version_id: String,
    pub version: i32,
    pub status: String,
}

pub struct CampaignVersionNewCommand;

impl CommandAction for CampaignVersionNewCommand {
    const NAME: &'static str = "campaignVersionNew";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignVersionNew");
    type Input = CampaignVersionNewInput;
    type Output = CampaignVersionNewOutput;
    const ROUTE: &'static str = "/api/campaigns/{id}/versions";
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

        let campaign_exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM campaigns WHERE organization_id = $1 AND id = $2",
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

        if campaign_exists.is_none() {
            return Err(V3Error::new(
                "campaignNotFound",
                None,
                "campaign not found",
                StatusCode::NOT_FOUND,
            ));
        }

        let latest_row = sqlx::query(
            "SELECT id, version, starts_at, ends_at, budget_thb, max_uses, stack_group, priority, segment_snapshot_id \
             FROM campaign_versions WHERE organization_id = $1 AND campaign_id = $2 \
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

        let has_open_draft: bool = sqlx::query_scalar(
            "SELECT EXISTS (\
                 SELECT 1 FROM campaign_versions \
                 WHERE organization_id = $1 AND campaign_id = $2 AND status IN ('DRAFT', 'SIMULATED')\
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

        if has_open_draft {
            return Err(V3Error::new(
                "staleVersion",
                None,
                "campaign already has an open draft or simulated version",
                StatusCode::CONFLICT,
            ));
        }

        let now = Utc::now();

        let (
            new_version_num,
            starts_at,
            ends_at,
            budget_thb,
            max_uses,
            stack_group,
            priority,
            segment_snapshot_id,
            prev_version_id,
        ) = match latest_row {
            Some(row) => {
                let v_num: i32 = row.get("version");
                let prev_id: Uuid = row.get("id");
                (
                    v_num + 1,
                    row.get::<chrono::DateTime<Utc>, _>("starts_at"),
                    row.get::<chrono::DateTime<Utc>, _>("ends_at"),
                    row.get::<Decimal, _>("budget_thb"),
                    row.get::<Option<i32>, _>("max_uses"),
                    row.get::<String, _>("stack_group"),
                    row.get::<i32, _>("priority"),
                    row.get::<Option<Uuid>, _>("segment_snapshot_id"),
                    Some(prev_id),
                )
            }
            None => (
                1,
                now,
                now + chrono::Duration::days(30),
                Decimal::ZERO,
                None,
                "DEFAULT".to_string(),
                0,
                None,
                None,
            ),
        };

        let new_version_id = Uuid::new_v4();
        let user_id = resolve_user_id(tx, who, org_id).await;

        sqlx::query(
            "INSERT INTO campaign_versions ( \
                 id, organization_id, campaign_id, version, status, starts_at, ends_at, budget_thb, \
                 max_uses, stack_group, priority, segment_snapshot_id, version_hash, approval_id, \
                 simulated_at, edited_at, created_by, created_at \
             ) VALUES ( \
                 $1, $2, $3, $4, 'DRAFT', $5, $6, $7, \
                 $8, $9, $10, $11, NULL, NULL, \
                 NULL, $12, $13, $12 \
             )",
        )
        .bind(new_version_id)
        .bind(org_id)
        .bind(campaign_id)
        .bind(new_version_num)
        .bind(starts_at)
        .bind(ends_at)
        .bind(budget_thb)
        .bind(max_uses)
        .bind(&stack_group)
        .bind(priority)
        .bind(segment_snapshot_id)
        .bind(now)
        .bind(user_id)
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

        if let Some(prev_id) = prev_version_id {
            sqlx::query(
                "INSERT INTO campaign_branches (campaign_version_id, branch_id) \
                 SELECT $1, branch_id FROM campaign_branches WHERE campaign_version_id = $2",
            )
            .bind(new_version_id)
            .bind(prev_id)
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

            sqlx::query(
                "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no) \
                 SELECT gen_random_uuid(), $1, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
                 FROM benefit_rules WHERE campaign_version_id = $2",
            )
            .bind(new_version_id)
            .bind(prev_id)
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

        sqlx::query(
            "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
             VALUES ($1, 0, 0)",
        )
        .bind(new_version_id)
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

        Ok(CampaignVersionNewOutput {
            version_id: new_version_id.to_string(),
            version: new_version_num,
            status: "DRAFT".to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CampaignVersionNewCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignVersionNewInput::export_all(&cfg)?;
    CampaignVersionNewOutput::export_all(&cfg)?;
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
