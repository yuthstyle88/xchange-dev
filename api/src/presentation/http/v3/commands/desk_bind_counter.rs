//! CA-1c (`docs/design/xc-counter-app.md` §5.4, §8.4): `POST /api/v3/commands/desk_bind_counter` binds the caller's
//! staff session to the counter named by a signed `DeskNonce`.

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
use crate::infrastructure::device_agents::desk_binding::DeskNonce;
use crate::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::device_agents::session_binding::PgSessionBinding;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_bind_counter.ts")]
pub struct DeskBindCounterInput {
    pub nonce: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_bind_counter.ts")]
pub struct DeskBindCounterOutput {
    pub counter_agent_id: String,
}

pub struct DeskBindCounterCommand {
    pool: PgPool,
}

impl DeskBindCounterCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for DeskBindCounterCommand {
    const NAME: &'static str = "deskBindCounter";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "deskBindCounter");
    type Input = DeskBindCounterInput;
    type Output = DeskBindCounterOutput;
    const ROUTE: &'static str = "/api/v3/commands/desk_bind_counter";
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
        let _who = caller
            .user
            .as_ref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
        let session_hash = caller
            .session_hash
            .as_deref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

        let keys = xc_customer::get_pii_keys().map_err(|_| server_error())?;
        let opened = DeskNonce::open(&input.nonce, &keys, chrono::Utc::now()).map_err(|r| {
            let code = r.code();
            V3Error::new(code, None, code, StatusCode::BAD_REQUEST)
        })?;

        let agent = if Uuid::parse_str(&opened.agent_id).is_ok() {
            PgAgentRepository::new(self.pool.clone())
                .find(&opened.agent_id)
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

        let bound = PgSessionBinding::new(self.pool.clone())
            .bind_in(tx, session_hash, &agent.id)
            .await
            .map_err(|_| server_error())?;
        if !bound {
            return Err(V3Error::from(PolicyError::SignIn));
        }

        Ok(DeskBindCounterOutput {
            counter_agent_id: agent.id,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(DeskBindCounterCommand::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeskBindCounterInput::export_all(&cfg)?;
    DeskBindCounterOutput::export_all(&cfg)?;
    Ok(())
}
