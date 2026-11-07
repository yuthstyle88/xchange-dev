//! Ticket CA-1d (`docs/design/xc-counter-app.md` §4, §5.2, §8.4): `POST /api/v3/commands/branch_counter_save`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, audit_role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::device_agents::agent_repository::{AgentRepository, DeviceAgent};
use crate::infrastructure::device_agents::counter_store::{CounterSettings, CounterStore};
use crate::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::branch_counters::BranchCounterRow;
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input::{self, TextBound};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_counters.ts")]
pub struct BranchCounterSaveInput {
    pub branch: String,
    pub agent_id: String,
    pub name: String,
    pub cash_drawer: Option<bool>,
    pub scanner: Option<bool>,
    pub kiosk_lock: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_counters.ts")]
pub struct BranchCounterSaveAnswer {
    pub counter: BranchCounterRow,
}

pub struct BranchCounterSaveCommand {
    pool: PgPool,
    agents: PgAgentRepository,
}

impl BranchCounterSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self {
            agents: PgAgentRepository::new(pool.clone()),
            pool,
        }
    }
}

fn settings_json(agent: &DeviceAgent) -> serde_json::Value {
    json!({
        "name": agent.counter_name,
        "cashDrawer": agent.cash_drawer,
        "scanner": agent.scanner,
        "kioskLock": agent.kiosk_lock,
    })
}

impl CommandAction for BranchCounterSaveCommand {
    const NAME: &'static str = "branchCounterSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "branchCounterSave");
    type Input = BranchCounterSaveInput;
    type Output = BranchCounterSaveAnswer;
    const ROUTE: &'static str = "/api/v3/commands/branch_counter_save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(V3Error::from(PolicyError::SignIn))
    }

    async fn run_as(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        caller: &Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let who = caller
            .user
            .as_ref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
        let name = input::text("name", &input.name, TextBound { min: 0, max: 40 })?.to_owned();
        let settings = CounterSettings {
            name,
            cash_drawer: input.cash_drawer,
            scanner: input.scanner,
            kiosk_lock: input.kiosk_lock,
        };
        let before = self
            .agents
            .active_agents_in(tx, branch)
            .await
            .map_err(|_| server_error())?
            .into_iter()
            .find(|a| a.id == input.agent_id);
        let saved = self
            .agents
            .save_in(tx, branch, &input.agent_id, &settings)
            .await
            .map_err(|_| server_error())?
            .ok_or_else(|| {
                V3Error::new(
                    "notFound",
                    Some("agentId"),
                    "counter not found",
                    StatusCode::NOT_FOUND,
                )
            })?;
        let before = before.ok_or_else(server_error)?;
        let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| server_error())?;
        PgAuditLog::new(self.pool.clone())
            .append_in(
                tx,
                &org_id.to_string(),
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: audit_role(&caller.granted, who, Utc::now()).to_owned(),
                    scope: "branch".to_owned(),
                    action: "BRANCH_COUNTER_SAVE".to_owned(),
                    object_ref: format!("counter:{}", saved.id),
                    branch_id: Some(branch.as_str().to_owned()),
                    before: Some(settings_json(&before)),
                    after: Some(settings_json(&saved)),
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "branch counter save audit failed");
                server_error()
            })?;
        Ok(BranchCounterSaveAnswer {
            counter: BranchCounterRow::from(saved),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(BranchCounterSaveCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchCounterSaveInput::export_all(&cfg)?;
    BranchCounterSaveAnswer::export_all(&cfg)?;
    Ok(())
}
