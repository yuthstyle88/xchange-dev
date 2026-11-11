//! xchange#6569 (`docs/design/xc-counter-app.md` §8.8): `POST /api/v3/commands/counter_unlock_issue` gives a manager
//! at the desk a one-time code that lets counter `agentId` leave kiosk mode. `CounterUnlock` (manager and owner only)
//! is checked by the policy row; the counter must be live and in the caller's session branch.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::device_agents::agent_repository::AgentRepository;
use crate::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::device_agents::unlock_tokens::PgUnlockTokens;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "counter_unlock_issue.ts")]
pub struct CounterUnlockIssueInput {
    pub agent_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "counter_unlock_issue.ts")]
pub struct CounterUnlockIssueOutput {
    pub code: String,
    pub expires_at: String,
}

pub struct CounterUnlockIssueCommand {
    pool: PgPool,
}

impl CounterUnlockIssueCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for CounterUnlockIssueCommand {
    const NAME: &'static str = "counterUnlockIssue";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "counterUnlockIssue");
    type Input = CounterUnlockIssueInput;
    type Output = CounterUnlockIssueOutput;
    const ROUTE: &'static str = "/api/v3/commands/counter_unlock_issue";
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
        _branch: &BranchId,
        caller: &Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let who = caller
            .user
            .as_ref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
        let session_hash = caller
            .session_hash
            .as_deref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

        let agent = if Uuid::parse_str(&input.agent_id).is_ok() {
            PgAgentRepository::new(self.pool.clone())
                .find(&input.agent_id)
                .await
                .map_err(|_| server_error())?
        } else {
            None
        };
        let Some(agent) = agent.filter(|a| a.revoked_at.is_none()) else {
            return Err(V3Error::new(
                "COUNTER_REVOKED",
                None,
                "COUNTER_REVOKED",
                StatusCode::CONFLICT,
            ));
        };

        let session_branch: Option<Option<String>> =
            sqlx::query_scalar("SELECT branch_id FROM user_sessions WHERE hash = $1")
                .bind(session_hash)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| server_error())?;
        let Some(session_branch) = session_branch else {
            return Err(V3Error::from(PolicyError::SignIn));
        };
        if session_branch.as_deref() != Some(agent.branch_id.as_str()) {
            return Err(V3Error::new(
                "COUNTER_OTHER_BRANCH",
                None,
                "COUNTER_OTHER_BRANCH",
                StatusCode::CONFLICT,
            ));
        }

        let issued = PgUnlockTokens::issue_in(tx, &agent.id, &who.user_id, chrono::Utc::now())
            .await
            .map_err(|_| server_error())?;
        tracing::info!(agent_id = %agent.id, issued_by = %who.user_id, "counter unlock code issued");

        Ok(CounterUnlockIssueOutput {
            code: issued.code,
            expires_at: issued.expires_at.to_rfc3339(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(CounterUnlockIssueCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CounterUnlockIssueInput::export_all(&cfg)?;
    CounterUnlockIssueOutput::export_all(&cfg)?;
    Ok(())
}
