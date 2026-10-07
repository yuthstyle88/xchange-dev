//! Agent repository trait: every statement on `device_agents`, and the agent's
//! changes to `device_jobs` (design §5.4c *Who writes `device_jobs.status`*).

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::infrastructure::devices::device_job::{DeviceJob, JobStatus};

type Tx<'a, 'c> = &'a mut sqlx::Transaction<'c, sqlx::Postgres>;

/// One counter drawer agent as stored. Neither hash is ever handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAgent {
    /// A UUID, lower case with dashes.
    pub id: String,
    pub branch_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    /// `None` once the code is used (or never set).
    pub pair_code_expires_at: Option<DateTime<Utc>>,
    pub paired_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub agent_version: Option<String>,
    pub agent_build: Option<String>,
    pub agent_os: Option<String>,
    pub agent_host: Option<String>,
    /// `CUPS` or `TCP`.
    pub drawer_transport: Option<String>,
    /// A CUPS queue name, or `host:port`.
    pub drawer_target: Option<String>,
}

/// An agent about to be stored: the owner's `add`.
#[derive(Debug, Clone)]
pub struct NewAgent<'a> {
    /// A UUID, lower case with dashes.
    pub id: &'a str,
    pub name: &'a str,
    pub created_by: &'a str,
    /// `AgentToken::code_hash` of the code shown once.
    pub pair_code_hash: &'a str,
    pub pair_code_expires_at: DateTime<Utc>,
    pub at: DateTime<Utc>,
}

/// The drawer an agent drives, as it reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawerTarget<'a> {
    /// `CUPS` or `TCP`.
    pub transport: &'a str,
    pub target: &'a str,
}

/// What an agent says about itself, at pairing (`drawer: None`, which leaves
/// the stored drawer as it is) and at every claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentSeen<'a> {
    pub version: &'a str,
    pub build: &'a str,
    pub os: &'a str,
    pub host: &'a str,
    pub drawer: Option<DrawerTarget<'a>>,
}

/// The agent's claim counter after counting one claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimWindow {
    /// Claims in the current window, this one included.
    pub count: i32,
    /// When the current window started; it lasts 60 seconds.
    pub window_start: DateTime<Utc>,
}

/// What one claim changed, in the order it changed it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaimStep {
    /// `QUEUED` drawer jobs created before `stale_before`, now `EXPIRED`.
    pub expired: Vec<DeviceJob>,
    /// `LEASED` jobs whose lease ended before `now`, now `FAILED` `LEASE_LAPSED`.
    pub lapsed: Vec<DeviceJob>,
    /// The oldest `QUEUED` drawer job left, now `LEASED` to the agent.
    pub leased: Option<DeviceJob>,
}

/// The arguments of one claim.
#[derive(Debug, Clone, Copy)]
pub struct ClaimArgs<'a> {
    pub agent_id: &'a str,
    /// A new UUID for the lease.
    pub lease_id: &'a str,
    pub now: DateTime<Utc>,
    /// `now − 10 s`: a drawer job created before it expires.
    pub stale_before: DateTime<Utc>,
    /// `now + 10 s`.
    pub lease_until: DateTime<Utc>,
}

/// A job with its lease columns and the reason it was queued with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLease {
    pub job: DeviceJob,
    pub lease_id: Option<String>,
    pub leased_by: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
    pub outcome_reason: Option<String>,
    /// `detail.reason` of the line written when the job was queued, if any.
    pub queue_reason: Option<String>,
}

pub trait AgentRepository: Send + Sync {
    /// Whether the branch has an agent that is not revoked (paired or not), as
    /// the transaction sees it.
    fn has_active_agent_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// The agent whose token hashes to `token_hash`, revoked or not.
    fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = AppResult<Option<DeviceAgent>>> + Send;

    /// One agent by id, revoked or not.
    fn find(&self, agent_id: &str) -> impl Future<Output = AppResult<Option<DeviceAgent>>> + Send;

    /// Stores a new, unpaired agent unless the branch already has an active
    /// one. `true` when it was stored.
    fn insert_agent(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        agent: &NewAgent<'_>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Gives an active agent of the branch a new pairing code (re-pair).
    /// `true` when such an agent exists.
    fn set_pairing(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        agent_id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// One statement: the active agent whose code hashes to `pair_code_hash`
    /// and has not expired at `now` takes `token_hash`, `paired_at = now`,
    /// `last_seen_at = now` and what it says about itself; its code is used up
    /// (both code columns `NULL`). `None` when no such agent.
    fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &AgentSeen<'_>,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<DeviceAgent>>> + Send;

    /// One statement: an active agent's `last_seen_at`, what it says about
    /// itself, and its claim counter (a window of 60 seconds). `None` when the
    /// agent is revoked or unknown.
    fn touch(
        &self,
        agent_id: &str,
        seen: &AgentSeen<'_>,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<ClaimWindow>>> + Send;

    /// The lock-free probe: does the branch have a `QUEUED` drawer job, or a
    /// `LEASED` job whose lease ended before `now`.
    fn has_claimable(
        &self,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Expire, lapse, lease (§5.4c *Claim* step 5), on the branch transaction.
    fn claim_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        args: &ClaimArgs<'_>,
    ) -> impl Future<Output = AppResult<ClaimStep>> + Send;

    /// A job of the branch with its lease columns.
    fn lease_of_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        job_id: &str,
    ) -> impl Future<Output = AppResult<Option<JobLease>>> + Send;

    /// One statement: moves the job from `from` to `to` under `lease_id`,
    /// storing `reason` as `outcome_reason`. With `from = Failed` it moves only
    /// a job whose `outcome_reason` is `LEASE_LAPSED`. `true` when it moved.
    #[allow(clippy::too_many_arguments)]
    fn report_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        job_id: &str,
        lease_id: &str,
        from: JobStatus,
        to: JobStatus,
        reason: Option<&str>,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    /// Revokes an active agent of the branch and fails the branch's `QUEUED`
    /// drawer jobs with `AGENT_REVOKED`; `LEASED` ones are left to lapse.
    /// `None` when the branch has no such active agent.
    fn revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        agent_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = AppResult<Option<Vec<DeviceJob>>>> + Send;

    /// Every agent, by branch id, then oldest first, then id.
    fn list(&self) -> impl Future<Output = AppResult<Vec<DeviceAgent>>> + Send;
}
