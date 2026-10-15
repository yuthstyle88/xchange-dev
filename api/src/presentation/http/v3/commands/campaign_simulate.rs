//! `campaignSimulate` (MK-4, design `marketing-campaigns.md` §4.5, §7).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_marketing::application::simulator::{CampaignSimulator, SimulationScenario};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_simulate.ts", rename_all = "camelCase")]
pub struct CampaignSimulateInput {
    pub campaign_id: Option<String>,
    #[serde(default)]
    #[ts(skip)]
    pub id: Option<String>,
    pub scenarios: Vec<CampaignSimulateScenarioDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_simulate.ts", rename_all = "camelCase")]
pub struct CampaignSimulateScenarioDto {
    pub branch_id: String,
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "campaign_simulate.ts", rename_all = "camelCase")]
pub struct CampaignSimulateOutput {
    pub simulation_id: String,
    pub cost_source: String,
}

pub struct CampaignSimulateCommand;

impl CommandAction for CampaignSimulateCommand {
    const NAME: &'static str = "campaignSimulate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "campaignSimulate");
    type Input = CampaignSimulateInput;
    type Output = CampaignSimulateOutput;
    const ROUTE: &'static str = "/api/campaigns/{id}/simulate";
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
        })?
        .ok_or_else(|| {
            V3Error::new(
                "versionNotFound",
                None,
                "no draft or simulated version found for campaign",
                StatusCode::NOT_FOUND,
            )
        })?;

        if input.scenarios.is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("scenarios"),
                "at least one scenario is required",
                StatusCode::BAD_REQUEST,
            ));
        }

        let mut scenarios = Vec::new();
        for s in input.scenarios {
            CurrencyCode::parse(&s.currency).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("currency"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
            let amount = Decimal::from_str(&s.amount).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("amount"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;
            scenarios.push(SimulationScenario {
                branch_id: s.branch_id,
                currency: s.currency,
                amount,
            });
        }

        let user_id = resolve_user_id(tx, who, org_id).await;
        let now = Utc::now();

        let sim_result = CampaignSimulator::simulate_in(
            tx,
            org_id,
            version_id,
            scenarios,
            user_id,
            now,
        )
        .await
        .map_err(|e| {
            V3Error::new(
                "simulationFailed",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        Ok(CampaignSimulateOutput {
            simulation_id: sim_result.simulation_id.to_string(),
            cost_source: sim_result.cost_source,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(CampaignSimulateCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CampaignSimulateScenarioDto::export_all(&cfg)?;
    CampaignSimulateInput::export_all(&cfg)?;
    CampaignSimulateOutput::export_all(&cfg)?;
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
