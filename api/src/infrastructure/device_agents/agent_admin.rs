//! The owner's agent administration (add, pair, revoke), filled by row AG-4 (design §5.4c *Identity*).

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use super::agent_repository::AgentRepository;
use super::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::paired_devices::pairing::{
    DevicePairing, PairingCode as CorePairingCode, PairingError,
};

const NO_SUCH_AGENT: &str = "ไม่พบเครื่องคุมลิ้นชัก";
const BRANCH_HAS_AGENT: &str = "สาขานี้มีเครื่องคุมลิ้นชักแล้ว";
const SPECIFY_AGENT_NAME: &str = "ระบุชื่อเครื่อง";
const AGENT_NAME_TOO_LONG: &str = "ชื่อเครื่องยาวได้ไม่เกิน 60 ตัวอักษร";

/// A pairing code returned by `add` or `new_code`. Shown once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub agent_id: String,
    pub code: String,
    pub code_expires_at: DateTime<Utc>,
}

impl From<CorePairingCode> for PairingCode {
    fn from(code: CorePairingCode) -> Self {
        Self {
            agent_id: code.device_id,
            code: code.code,
            code_expires_at: code.code_expires_at,
        }
    }
}

impl From<PairingError> for AppError {
    fn from(err: PairingError) -> Self {
        match err {
            PairingError::NameRequired => AppError::Domain(SPECIFY_AGENT_NAME.to_owned()),
            PairingError::NameTooLong => AppError::Domain(AGENT_NAME_TOO_LONG.to_owned()),
            PairingError::BranchHasOne => AppError::Domain(BRANCH_HAS_AGENT.to_owned()),
            PairingError::NoSuchDevice => AppError::Domain(NO_SUCH_AGENT.to_owned()),
            PairingError::App(e) => e,
        }
    }
}

/// An agent listed for the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedAgent {
    pub agent_id: String,
    pub branch_id: String,
    pub name: String,
    pub state: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
    pub build: Option<String>,
    pub os: Option<String>,
    pub host: Option<String>,
    pub drawer_transport: Option<String>,
    pub drawer_target: Option<String>,
    pub created_at: DateTime<Utc>,
    pub paired_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct AgentAdmin {
    pairing: DevicePairing<PgAgentRepository>,
}

impl AgentAdmin {
    pub fn new(branch_tx: BranchTx) -> Self {
        let pool = branch_tx.pool().clone();
        let agents = Arc::new(PgAgentRepository::new(pool.clone()));
        let attempts = PgAttemptCounter::new(pool);
        let pairing = DevicePairing::new(agents, branch_tx, attempts);
        Self { pairing }
    }

    pub fn new_with(
        agents: PgAgentRepository,
        _device_repo: PgDeviceRepository,
        _branches: BranchService,
        branch_tx: BranchTx,
    ) -> Self {
        let attempts = PgAttemptCounter::new(branch_tx.pool().clone());
        let pairing = DevicePairing::new(Arc::new(agents), branch_tx, attempts);
        Self { pairing }
    }

    /// The branch of `agent_id`, or `None` when `agent_id` is not a valid UUID or names no agent.
    pub async fn branch_of(&self, agent_id: &str) -> AppResult<Option<BranchId>> {
        self.pairing.branch_of(agent_id).await
    }

    /// Every agent with its calculated state:
    /// - `REVOKED` if revoked_at is set;
    /// - otherwise `PENDING` if never paired;
    /// - otherwise `ONLINE` if now - last_seen_at <= 60 s;
    /// - otherwise `OFFLINE`.
    pub async fn list(&self, now: DateTime<Utc>) -> AppResult<Vec<ListedAgent>> {
        let all = AgentRepository::list(self.pairing.store()).await?;
        let listed = all
            .into_iter()
            .map(|agent| {
                let state = if agent.revoked_at.is_some() {
                    "REVOKED"
                } else if agent.paired_at.is_none() {
                    "PENDING"
                } else if let Some(last_seen) = agent.last_seen_at {
                    if now.signed_duration_since(last_seen) <= Duration::seconds(60) {
                        "ONLINE"
                    } else {
                        "OFFLINE"
                    }
                } else {
                    "OFFLINE"
                };

                ListedAgent {
                    agent_id: agent.id,
                    branch_id: agent.branch_id,
                    name: agent.name,
                    state: state.to_owned(),
                    last_seen_at: agent.last_seen_at,
                    version: agent.agent_version,
                    build: agent.agent_build,
                    os: agent.agent_os,
                    host: agent.agent_host,
                    drawer_transport: agent.drawer_transport,
                    drawer_target: agent.drawer_target,
                    created_at: agent.created_at,
                    paired_at: agent.paired_at,
                    revoked_at: agent.revoked_at,
                }
            })
            .collect();
        Ok(listed)
    }

    /// Adds a new agent to `branch`, generating a 10-minute pairing code.
    /// The branch must exist and have no active agent.
    pub async fn add(
        &self,
        branch_id: &str,
        name: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<PairingCode> {
        let branch = BranchId::new(branch_id);
        let code = self.pairing.issue(&branch, name, actor, now).await?;
        Ok(code.into())
    }

    /// Gives an active agent of the branch a new pairing code (re-pair).
    pub async fn new_code(
        &self,
        agent_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<PairingCode> {
        let code = self.pairing.renew(agent_id, actor, now).await?;
        Ok(code.into())
    }

    /// Revokes an active agent of the branch and fails the branch's queued drawer jobs.
    pub async fn revoke(&self, agent_id: &str, actor: &str, now: DateTime<Utc>) -> AppResult<()> {
        self.pairing.revoke(agent_id, actor, now).await?;
        Ok(())
    }
}
