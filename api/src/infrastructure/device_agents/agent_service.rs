//! The agent service, filled by row AG-2 (design §5.4c *Claim*, *Record*).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;

use super::agent_repository::{AgentSeen, DeviceAgent};
use super::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::devices::printer_models::PrinterModels;
use crate::infrastructure::paired_devices::pairing::{
    AuthOutcome as CoreAuthOutcome, DevicePairing, PairOutcome as CorePairOutcome,
};

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
pub type AuthOutcome = CoreAuthOutcome<DeviceAgent>;

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
    pub(crate) branch_tx: BranchTx,
    pub(crate) printer_models: PrinterModels,
    pairing: DevicePairing<PgAgentRepository>,
}

impl AgentService {
    pub fn new(
        agent_repo: Arc<PgAgentRepository>,
        device_repo: Arc<PgDeviceRepository>,
        attempts: PgAttemptCounter,
        branch_tx: BranchTx,
        printer_models: PrinterModels,
    ) -> Self {
        let pairing = DevicePairing::new(agent_repo.clone(), branch_tx.clone(), attempts);
        Self {
            agent_repo,
            device_repo,
            branch_tx,
            printer_models,
            pairing,
        }
    }

    /// Authenticates a bearer token or records a failed attempt.
    pub async fn authenticate(
        &self,
        token_str: Option<&str>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<AuthOutcome> {
        self.pairing.authenticate(token_str, address, now).await
    }

    /// Pairs an agent with a one-time code.
    pub async fn pair(
        &self,
        params: &PairParams<'_>,
        address: &str,
        now: DateTime<Utc>,
    ) -> AppResult<PairOutcome> {
        let seen = AgentSeen {
            version: params.version,
            build: params.build,
            os: params.os,
            host: params.host,
            drawer: None,
        };
        match self.pairing.pair(params.code, &seen, address, now).await? {
            CorePairOutcome::Success { device, token } => Ok(PairOutcome::Success {
                agent_id: device.id,
                token,
                branch_id: device.branch_id,
                agent_name: device.name,
            }),
            CorePairOutcome::RateLimited => Ok(PairOutcome::RateLimited),
            CorePairOutcome::InvalidCode => Ok(PairOutcome::InvalidCode),
        }
    }
}
