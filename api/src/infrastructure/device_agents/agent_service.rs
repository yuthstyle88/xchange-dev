//! The agent service, filled by row AG-2 (design §5.4c *Claim*, *Record*).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;

use super::agent_repository::{AgentRepository, AgentSeen, DeviceAgent};
use super::agent_token::AgentToken;
use super::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::devices::printer_models::PrinterModels;

/// Outcome of a pairing attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairOutcome {
    Success {
        agent_id: String,
        token: String,
        branch_id: String,
        agent_name: String,
    },
    RateLimited,
    InvalidCode,
}

/// Outcome of bearer token authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Ok(Box<DeviceAgent>),
    RateLimited,
    InvalidToken,
}

/// The 15-minute window start for attempt rate-limiting.
fn window_start_15m(now: DateTime<Utc>) -> DateTime<Utc> {
    let millis = now.timestamp_millis();
    let window_millis = millis.div_euclid(15 * 60 * 1000) * (15 * 60 * 1000);
    DateTime::from_timestamp_millis(window_millis).unwrap_or(now)
}

/// Parameters for pairing an agent.
#[derive(Debug, Clone)]
pub struct PairParams<'a> {
    pub code: &'a str,
    pub host: &'a str,
    pub os: &'a str,
    pub version: &'a str,
    pub build: &'a str,
}

/// Coordinates device agent operations across repositories and transactions.
#[derive(Clone)]
pub struct AgentService {
    pub(crate) agent_repo: Arc<PgAgentRepository>,
    pub(crate) device_repo: Arc<PgDeviceRepository>,
    pub(crate) attempts: PgAttemptCounter,
    pub(crate) branch_tx: BranchTx,
    pub(crate) printer_models: PrinterModels,
}

impl AgentService {
    pub fn new(
        agent_repo: Arc<PgAgentRepository>,
        device_repo: Arc<PgDeviceRepository>,
        attempts: PgAttemptCounter,
        branch_tx: BranchTx,
        printer_models: PrinterModels,
    ) -> Self {
        Self {
            agent_repo,
            device_repo,
            attempts,
            branch_tx,
            printer_models,
        }
    }

    /// Authenticates a bearer token or records a failed attempt.
    pub async fn authenticate(
        &self,
        token_str: Option<&str>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<AuthOutcome> {
        let Some(raw_token) = token_str else {
            return self.record_failed_auth(address, now).await;
        };

        let token = raw_token.trim();
        if !token.starts_with("xcda_") {
            return self.record_failed_auth(address, now).await;
        }

        let token_hash = AgentToken::hash(token);
        let Some(agent) = self.agent_repo.find_by_token_hash(&token_hash).await? else {
            return self.record_failed_auth(address, now).await;
        };

        if agent.revoked_at.is_some() {
            return self.record_failed_auth(address, now).await;
        }

        Ok(AuthOutcome::Ok(Box::new(agent)))
    }

    async fn record_failed_auth(
        &self,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<AuthOutcome> {
        let key = AgentToken::hash(&format!("AGENT:{address}"));
        let win = window_start_15m(now);
        let count = self.attempts.increment(&key, win).await?;
        if count > 30 {
            Ok(AuthOutcome::RateLimited)
        } else {
            Ok(AuthOutcome::InvalidToken)
        }
    }

    /// Pairs an agent with a one-time code.
    pub async fn pair(
        &self,
        params: &PairParams<'_>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<PairOutcome> {
        let key = AgentToken::hash(&format!("PAIR:{address}"));
        let win = window_start_15m(now);
        let count = self.attempts.increment(&key, win).await?;
        if count > 10 {
            return Ok(PairOutcome::RateLimited);
        }

        let Some(code_hash) = AgentToken::code_hash(params.code) else {
            return Ok(PairOutcome::InvalidCode);
        };

        let token = AgentToken::new_token();
        let token_hash = AgentToken::hash(&token);
        let seen = AgentSeen {
            version: params.version,
            build: params.build,
            os: params.os,
            host: params.host,
            drawer: None,
        };

        let Some(agent) = self
            .agent_repo
            .pair(&code_hash, &token_hash, &seen, now)
            .await?
        else {
            return Ok(PairOutcome::InvalidCode);
        };

        Ok(PairOutcome::Success {
            agent_id: agent.id,
            token,
            branch_id: agent.branch_id,
            agent_name: agent.name,
        })
    }
}
