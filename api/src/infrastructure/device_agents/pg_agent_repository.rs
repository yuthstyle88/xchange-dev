//! PostgreSQL implementation of the agent repository (design §5.4c).
//!
//! Ids of `uuid` columns are bound as text with `$n::uuid` and read with `::text` (sqlx has
//! no `uuid` feature here).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use super::agent_repository::{
    AgentRepository, AgentSeen, ClaimArgs, ClaimStep, ClaimWindow, DeviceAgent, JobLease, NewAgent,
};
use crate::infrastructure::devices::device_job::{DeviceJob, JobKind, JobStatus};
use crate::infrastructure::devices::device_repository::{DeviceRepository, NewEvent};
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::paired_devices::kind::DeviceKind;
use crate::infrastructure::paired_devices::store::{
    NewPaired, PairedDevice, PairedDeviceStore, Tx,
};

#[derive(Clone)]
pub struct PgAgentRepository {
    pool: PgPool,
}

impl PgAgentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// The reason a lease that ended without a report is failed with; a late report may still
/// replace it.
const LEASE_LAPSED: &str = "LEASE_LAPSED";
/// The reason the branch's queued drawer jobs are failed with when its agent is revoked.
const AGENT_REVOKED: &str = "AGENT_REVOKED";

const AGENT_COLUMNS: &str = "id::text AS id, branch_id, name, created_at, created_by, \
     pair_code_expires_at, paired_at, revoked_at, revoked_by, last_seen_at, agent_version, \
     agent_build, agent_os, agent_host, drawer_transport, drawer_target";

const JOB_COLUMNS: &str =
    "id, request_id, branch_id, ref, kind, status, actor, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: String,
    branch_id: String,
    name: String,
    created_at: DateTime<Utc>,
    created_by: String,
    pair_code_expires_at: Option<DateTime<Utc>>,
    paired_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    revoked_by: Option<String>,
    last_seen_at: Option<DateTime<Utc>>,
    agent_version: Option<String>,
    agent_build: Option<String>,
    agent_os: Option<String>,
    agent_host: Option<String>,
    drawer_transport: Option<String>,
    drawer_target: Option<String>,
}

impl From<AgentRow> for DeviceAgent {
    fn from(r: AgentRow) -> Self {
        DeviceAgent {
            id: r.id,
            branch_id: r.branch_id,
            name: r.name,
            created_at: r.created_at,
            created_by: r.created_by,
            pair_code_expires_at: r.pair_code_expires_at,
            paired_at: r.paired_at,
            revoked_at: r.revoked_at,
            revoked_by: r.revoked_by,
            last_seen_at: r.last_seen_at,
            agent_version: r.agent_version,
            agent_build: r.agent_build,
            agent_os: r.agent_os,
            agent_host: r.agent_host,
            drawer_transport: r.drawer_transport,
            drawer_target: r.drawer_target,
        }
    }
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    request_id: String,
    branch_id: String,
    #[sqlx(rename = "ref")]
    reference: String,
    kind: String,
    status: String,
    actor: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl JobRow {
    fn into_job(self) -> AppResult<DeviceJob> {
        let kind = JobKind::parse(&self.kind).ok_or_else(|| {
            AppError::Internal(format!("device job {}: kind {}", self.id, self.kind))
        })?;
        let status = JobStatus::parse(&self.status).ok_or_else(|| {
            AppError::Internal(format!("device job {}: status {}", self.id, self.status))
        })?;
        Ok(DeviceJob {
            id: self.id,
            request_id: self.request_id,
            branch_id: self.branch_id,
            reference: self.reference,
            kind,
            status,
            actor: self.actor,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct LeaseRow {
    #[sqlx(flatten)]
    job: JobRow,
    lease_id: Option<String>,
    leased_by: Option<String>,
    lease_until: Option<DateTime<Utc>>,
    outcome_reason: Option<String>,
    queue_reason: Option<String>,
}

/// Jobs as `RETURNING` hands them back (in no order), oldest first, then by id.
fn ordered(rows: Vec<JobRow>) -> AppResult<Vec<DeviceJob>> {
    let mut jobs = rows
        .into_iter()
        .map(JobRow::into_job)
        .collect::<AppResult<Vec<_>>>()?;
    jobs.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    Ok(jobs)
}

impl AgentRepository for PgAgentRepository {
    async fn has_active_agent_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<bool> {
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM device_agents \
             WHERE branch_id = $1 AND revoked_at IS NULL)",
        )
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await?;
        Ok(found)
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> AppResult<Option<DeviceAgent>> {
        let row: Option<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {AGENT_COLUMNS} FROM device_agents WHERE token_hash = $1"
        )))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(DeviceAgent::from))
    }

    async fn find(&self, agent_id: &str) -> AppResult<Option<DeviceAgent>> {
        let row: Option<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {AGENT_COLUMNS} FROM device_agents WHERE id = $1::uuid"
        )))
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(DeviceAgent::from))
    }

    async fn insert_agent(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        agent: &NewAgent<'_>,
    ) -> AppResult<bool> {
        // The unique partial index decides: while the branch has an active agent (a PENDING
        // one counts) nothing is written.
        let done = sqlx::query(
            "INSERT INTO device_agents \
             (id, branch_id, name, created_at, created_by, pair_code_hash, pair_code_expires_at) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (branch_id) WHERE revoked_at IS NULL DO NOTHING",
        )
        .bind(agent.id)
        .bind(branch.as_str())
        .bind(agent.name)
        .bind(agent.at)
        .bind(agent.created_by)
        .bind(agent.pair_code_hash)
        .bind(agent.pair_code_expires_at)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn set_pairing(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        agent_id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "UPDATE device_agents SET pair_code_hash = $3, pair_code_expires_at = $4 \
             WHERE branch_id = $1 AND id = $2::uuid AND revoked_at IS NULL",
        )
        .bind(branch.as_str())
        .bind(agent_id)
        .bind(pair_code_hash)
        .bind(expires_at)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &AgentSeen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<DeviceAgent>> {
        // Single use: the code columns are cleared by the same statement that takes the token.
        let row: Option<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_agents SET token_hash = $2, paired_at = $3, last_seen_at = $3, \
             pair_code_hash = NULL, pair_code_expires_at = NULL, \
             agent_version = $4, agent_build = $5, agent_os = $6, agent_host = $7, \
             drawer_transport = COALESCE($8, drawer_transport), \
             drawer_target = COALESCE($9, drawer_target) \
             WHERE pair_code_hash = $1 AND revoked_at IS NULL AND pair_code_expires_at > $3 \
             RETURNING {AGENT_COLUMNS}"
        )))
        .bind(pair_code_hash)
        .bind(token_hash)
        .bind(now)
        .bind(seen.version)
        .bind(seen.build)
        .bind(seen.os)
        .bind(seen.host)
        .bind(seen.drawer.map(|d| d.transport))
        .bind(seen.drawer.map(|d| d.target))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(DeviceAgent::from))
    }

    async fn touch(
        &self,
        agent_id: &str,
        seen: &AgentSeen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<ClaimWindow>> {
        // A window lasts 60 s: a claim at or after `window_start + 60 s` opens a new one.
        let row: Option<(i32, DateTime<Utc>)> = sqlx::query_as(
            "UPDATE device_agents SET last_seen_at = $2, \
             agent_version = $3, agent_build = $4, agent_os = $5, agent_host = $6, \
             drawer_transport = COALESCE($7, drawer_transport), \
             drawer_target = COALESCE($8, drawer_target), \
             claim_count = CASE \
                 WHEN claim_window_start IS NULL \
                   OR claim_window_start + interval '60 seconds' <= $2 THEN 1 \
                 ELSE claim_count + 1 END, \
             claim_window_start = CASE \
                 WHEN claim_window_start IS NULL \
                   OR claim_window_start + interval '60 seconds' <= $2 THEN $2 \
                 ELSE claim_window_start END \
             WHERE id = $1::uuid AND revoked_at IS NULL \
             RETURNING claim_count, claim_window_start",
        )
        .bind(agent_id)
        .bind(now)
        .bind(seen.version)
        .bind(seen.build)
        .bind(seen.os)
        .bind(seen.host)
        .bind(seen.drawer.map(|d| d.transport))
        .bind(seen.drawer.map(|d| d.target))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(count, window_start)| ClaimWindow {
            count,
            window_start,
        }))
    }

    async fn has_claimable(&self, branch: &BranchId, now: DateTime<Utc>) -> AppResult<bool> {
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM device_jobs \
                 WHERE branch_id = $1 AND status = 'QUEUED' AND kind = 'DRAWER_REQUEST') \
             OR EXISTS (SELECT 1 FROM device_jobs \
                 WHERE branch_id = $1 AND status = 'LEASED' AND lease_until < $2)",
        )
        .bind(branch.as_str())
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(found)
    }

    async fn claim_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        args: &ClaimArgs<'_>,
    ) -> AppResult<ClaimStep> {
        let expired: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_jobs SET status = 'EXPIRED', updated_at = $2 \
             WHERE branch_id = $1 AND status = 'QUEUED' AND kind = 'DRAWER_REQUEST' \
             AND created_at < $3 \
             RETURNING {JOB_COLUMNS}"
        )))
        .bind(branch.as_str())
        .bind(args.now)
        .bind(args.stale_before)
        .fetch_all(&mut **tx)
        .await?;

        // The lease id is kept, so a late report under it can still be matched.
        let lapsed: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_jobs SET status = 'FAILED', outcome_reason = $3, updated_at = $2 \
             WHERE branch_id = $1 AND status = 'LEASED' AND lease_until < $2 \
             RETURNING {JOB_COLUMNS}"
        )))
        .bind(branch.as_str())
        .bind(args.now)
        .bind(LEASE_LAPSED)
        .fetch_all(&mut **tx)
        .await?;

        let leased: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_jobs SET status = 'LEASED', lease_id = $3::uuid, \
             leased_by = $4::uuid, lease_until = $5, updated_at = $2 \
             WHERE branch_id = $1 AND id = ( \
                 SELECT id FROM device_jobs \
                 WHERE branch_id = $1 AND status = 'QUEUED' AND kind = 'DRAWER_REQUEST' \
                 ORDER BY created_at, id LIMIT 1 FOR UPDATE) \
             RETURNING {JOB_COLUMNS}"
        )))
        .bind(branch.as_str())
        .bind(args.now)
        .bind(args.lease_id)
        .bind(args.agent_id)
        .bind(args.lease_until)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(ClaimStep {
            expired: ordered(expired)?,
            lapsed: ordered(lapsed)?,
            leased: leased.map(JobRow::into_job).transpose()?,
        })
    }

    async fn lease_of_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        job_id: &str,
    ) -> AppResult<Option<JobLease>> {
        // `device_jobs` has no reason column: the reason is on the job's queue line.
        let row: Option<LeaseRow> = sqlx::query_as(
            "SELECT j.id, j.request_id, j.branch_id, j.ref, j.kind, j.status, j.actor, \
             j.created_at, j.updated_at, j.lease_id::text AS lease_id, \
             j.leased_by::text AS leased_by, j.lease_until, j.outcome_reason, \
             (SELECT e.detail->>'reason' FROM device_events e \
                 WHERE e.branch_id = j.branch_id AND e.kind = j.kind \
                 AND e.detail->>'job' = j.id \
                 ORDER BY e.seq LIMIT 1) AS queue_reason \
             FROM device_jobs j WHERE j.branch_id = $1 AND j.id = $2",
        )
        .bind(branch.as_str())
        .bind(job_id)
        .fetch_optional(&mut **tx)
        .await?;
        row.map(|r| {
            Ok(JobLease {
                job: r.job.into_job()?,
                lease_id: r.lease_id,
                leased_by: r.leased_by,
                lease_until: r.lease_until,
                outcome_reason: r.outcome_reason,
                queue_reason: r.queue_reason,
            })
        })
        .transpose()
    }

    async fn report_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        job_id: &str,
        lease_id: &str,
        from: JobStatus,
        to: JobStatus,
        reason: Option<&str>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        // From FAILED only a lapse may be replaced (a late report); any other failure stands.
        let done = sqlx::query(
            "UPDATE device_jobs SET status = $5, outcome_reason = $6, updated_at = $7 \
             WHERE branch_id = $1 AND id = $2 AND lease_id = $3::uuid AND status = $4 \
             AND ($4 <> 'FAILED' OR outcome_reason = $8)",
        )
        .bind(branch.as_str())
        .bind(job_id)
        .bind(lease_id)
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(reason)
        .bind(now)
        .bind(LEASE_LAPSED)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn revoke_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        agent_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Vec<DeviceJob>>> {
        let revoked = sqlx::query(
            "UPDATE device_agents SET revoked_at = $4, revoked_by = $3 \
             WHERE branch_id = $1 AND id = $2::uuid AND revoked_at IS NULL",
        )
        .bind(branch.as_str())
        .bind(agent_id)
        .bind(actor)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        if revoked.rows_affected() == 0 {
            return Ok(None);
        }
        // Leased jobs are left to lapse; only the queue that no agent will now take fails.
        let failed: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_jobs SET status = 'FAILED', outcome_reason = $3, updated_at = $2 \
             WHERE branch_id = $1 AND status = 'QUEUED' AND kind = 'DRAWER_REQUEST' \
             RETURNING {JOB_COLUMNS}"
        )))
        .bind(branch.as_str())
        .bind(now)
        .bind(AGENT_REVOKED)
        .fetch_all(&mut **tx)
        .await?;
        ordered(failed).map(Some)
    }

    async fn list(&self) -> AppResult<Vec<DeviceAgent>> {
        let rows: Vec<AgentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {AGENT_COLUMNS} FROM device_agents ORDER BY branch_id, created_at, id"
        )))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(DeviceAgent::from).collect())
    }
}

impl PairedDevice for DeviceAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn branch_id(&self) -> &str {
        &self.branch_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

impl PairedDeviceStore for PgAgentRepository {
    const KIND: DeviceKind = DeviceKind::DrawerAgent;
    type Device = DeviceAgent;
    type Seen<'a> = AgentSeen<'a>;

    async fn insert_pending_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        device: &NewPaired<'_>,
    ) -> AppResult<bool> {
        let new_agent = NewAgent {
            id: device.id,
            name: device.name,
            created_by: device.created_by,
            pair_code_hash: device.pair_code_hash,
            pair_code_expires_at: device.pair_code_expires_at,
            at: device.at,
        };
        AgentRepository::insert_agent(self, tx, branch, &new_agent).await
    }

    async fn set_code_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        pair_code_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        AgentRepository::set_pairing(self, tx, branch, id, pair_code_hash, expires_at).await
    }

    async fn pair(
        &self,
        pair_code_hash: &str,
        token_hash: &str,
        seen: &Self::Seen<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<Option<DeviceAgent>> {
        AgentRepository::pair(self, pair_code_hash, token_hash, seen, now).await
    }

    async fn find_by_token_hash(&self, token_hash: &str) -> AppResult<Option<DeviceAgent>> {
        AgentRepository::find_by_token_hash(self, token_hash).await
    }

    async fn find(&self, id: &str) -> AppResult<Option<DeviceAgent>> {
        AgentRepository::find(self, id).await
    }

    async fn revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "UPDATE device_agents SET revoked_at = $4, revoked_by = $3 \
             WHERE branch_id = $1 AND id = $2::uuid AND revoked_at IS NULL",
        )
        .bind(branch.as_str())
        .bind(id)
        .bind(actor)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn after_revoke_in(
        &self,
        tx: Tx<'_, '_>,
        branch: &BranchId,
        _id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let failed: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE device_jobs SET status = 'FAILED', outcome_reason = $3, updated_at = $2 \
             WHERE branch_id = $1 AND status = 'QUEUED' AND kind = 'DRAWER_REQUEST' \
             RETURNING {JOB_COLUMNS}"
        )))
        .bind(branch.as_str())
        .bind(now)
        .bind(AGENT_REVOKED)
        .fetch_all(&mut **tx)
        .await?;
        let device_repo = PgDeviceRepository::new(self.pool.clone());
        for job in ordered(failed)? {
            let detail = serde_json::json!({
                "job": job.id,
                "reason": AGENT_REVOKED,
            });
            let line = NewEvent {
                dedupe: None,
                actor,
                kind: "FAILED",
                reference: &job.reference,
                detail: &detail,
                amounts: &[],
                at: now,
            };
            device_repo.insert_event(tx, branch, &line).await?;
        }
        Ok(())
    }

    async fn list(&self) -> AppResult<Vec<DeviceAgent>> {
        AgentRepository::list(self).await
    }
}
