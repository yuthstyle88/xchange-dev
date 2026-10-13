//! `campaignSubmit` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_marketing::domain::campaign::version::{CampaignVersion, StoredStatus, VersionTerms};
use xc_marketing::domain::campaign::version_hash::VersionHash;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_submit.ts", rename_all = "camelCase")]
pub struct CampaignSubmitInput {
    pub campaign_id: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_submit.ts", rename_all = "camelCase")]
pub struct CampaignSubmitOutput {
    pub version_id: String,
    pub status: String,
    pub version_hash: Option<String>,
}

pub struct CampaignSubmitCommand;

impl CommandAction for CampaignSubmitCommand {
    const NAME: &'static str = "campaignSubmit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignSubmit");
    type Input = CampaignSubmitInput;
    type Output = CampaignSubmitOutput;
    const ROUTE: &'static str = "/api/campaigns/{id}/submit";
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

        let version_row = sqlx::query(
            "SELECT id, organization_id, campaign_id, version, status, starts_at, ends_at, \
                    budget_thb, max_uses, stack_group, priority, segment_snapshot_id, \
                    edited_at, created_by, created_at \
             FROM campaign_versions \
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

        let Some(row) = version_row else {
            return Err(V3Error::new(
                "versionNotFound",
                None,
                "no draft or simulated version found for campaign",
                StatusCode::NOT_FOUND,
            ));
        };

        let version_id: Uuid = row.get("id");
        let edited_at: chrono::DateTime<Utc> = row.get("edited_at");

        // D-4: Check newest simulation in campaign_simulations: must exist and simulated_at >= edited_at
        let last_sim_at: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            "SELECT simulated_at FROM campaign_simulations \
             WHERE campaign_version_id = $1 \
             ORDER BY simulated_at DESC LIMIT 1",
        )
        .bind(version_id)
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

        let Some(sim_at) = last_sim_at else {
            return Err(V3Error::new(
                "simulationRequired",
                None,
                "simulation required before submit",
                StatusCode::BAD_REQUEST,
            ));
        };

        if sim_at < edited_at {
            return Err(V3Error::new(
                "simulationRequired",
                None,
                "simulation required after edits",
                StatusCode::BAD_REQUEST,
            ));
        }

        let version_num: i32 = row.get("version");
        let created_by: Uuid = row.get("created_by");
        let created_at: chrono::DateTime<Utc> = row.get("created_at");

        let terms = VersionTerms {
            starts_at: row.get("starts_at"),
            ends_at: row.get("ends_at"),
            budget_thb: row.get("budget_thb"),
            max_uses: row.get("max_uses"),
            stack_group: row.get("stack_group"),
            priority: row.get("priority"),
            segment_snapshot_id: row.get("segment_snapshot_id"),
        };

        let temp_version = CampaignVersion {
            id: version_id,
            organization_id: org_id,
            campaign_id,
            version: version_num,
            status: StoredStatus::Draft,
            terms,
            version_hash: None,
            approval_id: None,
            simulated_at: Some(sim_at),
            edited_at,
            created_by,
            created_at,
        };

        let rule_rows = sqlx::query(
            "SELECT benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no \
             FROM benefit_rules WHERE campaign_version_id = $1 ORDER BY rule_no ASC",
        )
        .bind(version_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| {
            V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let mut rule_blobs = Vec::new();
        for r in rule_rows {
            let b_type: String = r.get("benefit_type");
            let b_class: String = r.get("benefit_class");
            let elig: serde_json::Value = r.get("eligibility");
            let calc: serde_json::Value = r.get("calculation");
            let caps: serde_json::Value = r.get("caps");
            let req_c: bool = r.get("requires_coupon");
            let rule_no: i32 = r.get("rule_no");
            let blob = serde_json::json!({
                "ruleNo": rule_no,
                "benefitType": b_type,
                "benefitClass": b_class,
                "eligibility": elig,
                "calculation": calc,
                "caps": caps,
                "requiresCoupon": req_c,
            });
            rule_blobs.push(blob.to_string());
        }

        let branch_rows: Vec<String> = sqlx::query_scalar(
            "SELECT branch_id FROM campaign_branches WHERE campaign_version_id = $1 ORDER BY branch_id ASC",
        )
        .bind(version_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| {
            V3Error::new(
                "database",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let hash = VersionHash::of(&temp_version, &rule_blobs, &branch_rows);

        let approval_id = Uuid::new_v4();
        let now = Utc::now();
        let user_id = Uuid::parse_str(&who.user_id).unwrap_or(org_id);
        let idem_key = format!("campaign_version_submit_{version_id}");

        sqlx::query(
            "INSERT INTO approval_requests ( \
                 id, organization_id, type, branch_id, object_id, object_version_hash, \
                 policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key \
             ) VALUES ($1, $2, 'CAMPAIGN_VERSION', NULL, $3, $4, '1', '{}'::jsonb, $5, $6, 'PENDING', $7)",
        )
        .bind(approval_id)
        .bind(org_id)
        .bind(version_id.to_string())
        .bind(&hash)
        .bind(user_id.to_string())
        .bind(now)
        .bind(&idem_key)
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
            "UPDATE campaign_versions SET \
                 status = 'PENDING_APPROVAL', \
                 version_hash = $1, \
                 approval_id = $2, \
                 edited_at = $3 \
             WHERE id = $4",
        )
        .bind(&hash)
        .bind(approval_id)
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

        Ok(CampaignSubmitOutput {
            version_id: version_id.to_string(),
            status: "PENDING_APPROVAL".to_string(),
            version_hash: Some(hash),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CampaignSubmitCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignSubmitInput::export_all(&cfg)?;
    CampaignSubmitOutput::export_all(&cfg)?;
    Ok(())
}
