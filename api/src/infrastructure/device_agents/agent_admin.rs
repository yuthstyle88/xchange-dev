//! The owner's agent administration (add, pair, revoke), filled by row AG-4 (design §5.4c *Identity*).

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use uuid::Uuid;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use super::agent_repository::{AgentRepository, NewAgent};
use super::agent_token::AgentToken;
use super::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::devices::device_repository::{DeviceRepository, NewEvent};
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;

const NO_SUCH_AGENT: &str = "ไม่พบเครื่องคุมลิ้นชัก";
const BRANCH_HAS_AGENT: &str = "สาขานี้มีเครื่องคุมลิ้นชักแล้ว";
const SPECIFY_AGENT_NAME: &str = "ระบุชื่อเครื่อง";

/// A pairing code returned by `add` or `new_code`. Shown once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub agent_id: String,
    pub code: String,
    pub code_expires_at: DateTime<Utc>,
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
    agents: PgAgentRepository,
    device_repo: PgDeviceRepository,
    branches: BranchService,
    branch_tx: BranchTx,
}

impl AgentAdmin {
    pub fn new(branch_tx: BranchTx) -> Self {
        let pool = branch_tx.pool().clone();
        let agents = PgAgentRepository::new(pool.clone());
        let device_repo = PgDeviceRepository::new(pool);
        let branches = BranchService::new(branch_tx.clone());
        Self {
            agents,
            device_repo,
            branches,
            branch_tx,
        }
    }

    pub fn new_with(
        agents: PgAgentRepository,
        device_repo: PgDeviceRepository,
        branches: BranchService,
        branch_tx: BranchTx,
    ) -> Self {
        Self {
            agents,
            device_repo,
            branches,
            branch_tx,
        }
    }

    /// The branch of `agent_id`, or `None` when `agent_id` is not a valid UUID or names no agent.
    pub async fn branch_of(&self, agent_id: &str) -> AppResult<Option<BranchId>> {
        let Ok(uuid) = Uuid::parse_str(agent_id) else {
            return Ok(None);
        };
        let id_str = uuid.to_string();
        let Some(agent) = self.agents.find(&id_str).await? else {
            return Ok(None);
        };
        Ok(Some(BranchId::new(agent.branch_id)))
    }

    /// Every agent with its calculated state:
    /// - `REVOKED` if revoked_at is set;
    /// - otherwise `PENDING` if never paired;
    /// - otherwise `ONLINE` if now - last_seen_at <= 60 s;
    /// - otherwise `OFFLINE`.
    pub async fn list(&self, now: DateTime<Utc>) -> AppResult<Vec<ListedAgent>> {
        let all = self.agents.list().await?;
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
        self.branches.require(&branch).await?;

        let trimmed_name = name.trim();
        let units = trimmed_name.encode_utf16().count();
        if units == 0 || units > 60 {
            return Err(AppError::Domain(SPECIFY_AGENT_NAME.to_owned()));
        }

        let id = Uuid::new_v4().to_string();
        let code = AgentToken::new_pairing_code();
        let code_hash = AgentToken::code_hash(&code)
            .ok_or_else(|| AppError::Internal("failed to hash pairing code".to_owned()))?;
        let expires_at = now + Duration::minutes(10);

        let agents = self.agents.clone();
        let b = branch.clone();
        let id_clone = id.clone();
        let name_string = trimmed_name.to_owned();
        let actor_string = actor.to_owned();

        let stored = self
            .branch_tx
            .run(&branch, move |tx| {
                let agents = agents.clone();
                let b = b.clone();
                let id_clone = id_clone.clone();
                let name_string = name_string.clone();
                let actor_string = actor_string.clone();
                let code_hash = code_hash.clone();
                Box::pin(async move {
                    let new_agent = NewAgent {
                        id: &id_clone,
                        name: &name_string,
                        created_by: &actor_string,
                        pair_code_hash: &code_hash,
                        pair_code_expires_at: expires_at,
                        at: now,
                    };
                    agents.insert_agent(tx, &b, &new_agent).await
                })
            })
            .await?;

        if !stored {
            return Err(AppError::Domain(BRANCH_HAS_AGENT.to_owned()));
        }

        Ok(PairingCode {
            agent_id: id,
            code,
            code_expires_at: expires_at,
        })
    }

    /// Gives an active agent of the branch a new pairing code (re-pair).
    pub async fn new_code(
        &self,
        agent_id: &str,
        _actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<PairingCode> {
        let Ok(uuid) = Uuid::parse_str(agent_id) else {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        };
        let id_str = uuid.to_string();
        let Some(agent) = self.agents.find(&id_str).await? else {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        };
        if agent.revoked_at.is_some() {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        }

        let branch = BranchId::new(agent.branch_id);
        let code = AgentToken::new_pairing_code();
        let code_hash = AgentToken::code_hash(&code)
            .ok_or_else(|| AppError::Internal("failed to hash pairing code".to_owned()))?;
        let expires_at = now + Duration::minutes(10);

        let agents = self.agents.clone();
        let b = branch.clone();
        let id_clone = id_str.clone();
        let hash_clone = code_hash.clone();

        let updated = self
            .branch_tx
            .run(&branch, move |tx| {
                let agents = agents.clone();
                let b = b.clone();
                let id_clone = id_clone.clone();
                let hash_clone = hash_clone.clone();
                Box::pin(async move {
                    agents
                        .set_pairing(tx, &b, &id_clone, &hash_clone, expires_at)
                        .await
                })
            })
            .await?;

        if !updated {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        }

        Ok(PairingCode {
            agent_id: id_str,
            code,
            code_expires_at: expires_at,
        })
    }

    /// Revokes an active agent of the branch and fails the branch's queued drawer jobs.
    pub async fn revoke(&self, agent_id: &str, actor: &str, now: DateTime<Utc>) -> AppResult<()> {
        let Ok(uuid) = Uuid::parse_str(agent_id) else {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        };
        let id_str = uuid.to_string();
        let Some(agent) = self.agents.find(&id_str).await? else {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        };
        if agent.revoked_at.is_some() {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        }

        let branch = BranchId::new(agent.branch_id);
        let agents = self.agents.clone();
        let device_repo = self.device_repo.clone();
        let b = branch.clone();
        let id_clone = id_str.clone();
        let actor_str = actor.to_owned();

        let revoked = self
            .branch_tx
            .run(&branch, move |tx| {
                let agents = agents.clone();
                let device_repo = device_repo.clone();
                let b = b.clone();
                let id_clone = id_clone.clone();
                let actor_str = actor_str.clone();
                Box::pin(async move {
                    let Some(failed_jobs) =
                        agents.revoke_in(tx, &b, &id_clone, &actor_str, now).await?
                    else {
                        return Ok(false);
                    };
                    for job in failed_jobs {
                        let detail = serde_json::json!({
                            "job": job.id,
                            "reason": "AGENT_REVOKED",
                        });
                        let line = NewEvent {
                            dedupe: None,
                            actor: &actor_str,
                            kind: "FAILED",
                            reference: &job.reference,
                            detail: &detail,
                            amounts: &[],
                            at: now,
                        };
                        device_repo.insert_event(tx, &b, &line).await?;
                    }
                    Ok(true)
                })
            })
            .await?;

        if !revoked {
            return Err(AppError::Domain(NO_SUCH_AGENT.to_owned()));
        }

        Ok(())
    }
}
