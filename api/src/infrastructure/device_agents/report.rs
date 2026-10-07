//! The report block of the agent service, filled by row AG-2 (design §5.4c *Record*).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::agent_repository::{AgentRepository, DeviceAgent};
use super::agent_service::AgentService;
use crate::infrastructure::devices::device_job::JobStatus;
use crate::infrastructure::devices::device_repository::{DeviceRepository, NewEvent};

/// Outcome of a report call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportOutcome {
    Recorded,
    AlreadyRecorded,
    NotFound,
    Conflict,
    InvalidOutcome,
}

/// Parameters for reporting a job outcome.
#[derive(Debug, Clone)]
pub struct ReportParams<'a> {
    pub job_id: &'a str,
    pub lease_id: &'a str,
    pub outcome: &'a str,
    pub reason: &'a str,
    pub detail: &'a serde_json::Value,
}

impl AgentService {
    /// Reports the outcome of a leased drawer job.
    pub async fn report(
        &self,
        agent: &DeviceAgent,
        params: &ReportParams<'_>,
        now: DateTime<Utc>,
    ) -> AppResult<ReportOutcome> {
        let outcome = params.outcome;
        if outcome != "DRAWER_OPENED" && outcome != "FAILED" && outcome != "EXPIRED" {
            return Ok(ReportOutcome::InvalidOutcome);
        }

        let branch = BranchId::new(&agent.branch_id);
        let agent_repo = Arc::clone(&self.agent_repo);
        let device_repo = Arc::clone(&self.device_repo);
        let agent_id = agent.id.clone();
        let agent_name = agent.name.clone();
        let agent_version = agent.agent_version.clone().unwrap_or_default();
        let drawer_target = agent.drawer_target.clone().unwrap_or_default();
        let job_id = params.job_id.to_string();
        let lease_id = params.lease_id.to_string();
        let outcome = outcome.to_string();
        let reason = params.reason.to_string();
        let detail_val = params.detail.clone();
        let b = branch.clone();

        self.branch_tx
            .run(&b, move |tx| {
                let branch = branch.clone();
                let agent_repo = Arc::clone(&agent_repo);
                let device_repo = Arc::clone(&device_repo);
                let agent_id = agent_id.clone();
                let agent_name = agent_name.clone();
                let agent_version = agent_version.clone();
                let drawer_target = drawer_target.clone();
                let job_id = job_id.clone();
                let lease_id = lease_id.clone();
                let outcome = outcome.clone();
                let reason = reason.clone();
                let detail_val = detail_val.clone();

                Box::pin(async move {
                    let Some(job_lease) = agent_repo.lease_of_in(tx, &branch, &job_id).await?
                    else {
                        return Ok(ReportOutcome::NotFound);
                    };

                    if job_lease.leased_by.as_deref() != Some(&agent_id)
                        || job_lease.lease_id.as_deref() != Some(&lease_id)
                    {
                        return Ok(ReportOutcome::Conflict);
                    }

                    let lapsed = job_lease.job.status == JobStatus::Failed
                        && job_lease.outcome_reason.as_deref() == Some("LEASE_LAPSED");

                    if job_lease.job.status.as_str() == outcome && !lapsed {
                        return Ok(ReportOutcome::AlreadyRecorded);
                    }

                    let from = if job_lease.job.status == JobStatus::Leased {
                        JobStatus::Leased
                    } else if lapsed {
                        JobStatus::Failed
                    } else {
                        return Ok(ReportOutcome::Conflict);
                    };

                    let to = match outcome.as_str() {
                        "DRAWER_OPENED" => JobStatus::DrawerOpened,
                        "FAILED" => JobStatus::Failed,
                        "EXPIRED" => JobStatus::Expired,
                        _ => unreachable!(),
                    };

                    let reason_opt = if reason.trim().is_empty() {
                        None
                    } else {
                        Some(reason.as_str())
                    };

                    let moved = agent_repo
                        .report_in(tx, &branch, &job_id, &lease_id, from, to, reason_opt, now)
                        .await?;
                    if !moved {
                        return Ok(ReportOutcome::Conflict);
                    }

                    let actor = format!("agent:{agent_id}");
                    let queue_reason = job_lease.queue_reason.unwrap_or_default();
                    let cups_job = detail_val
                        .get("cupsJob")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let leased_at = job_lease
                        .lease_until
                        .map(|u| u - chrono::Duration::seconds(10))
                        .unwrap_or(now)
                        .to_rfc3339();
                    let physical_action = outcome == "DRAWER_OPENED";
                    let physical_open_confirmed = false;

                    let event_detail = serde_json::json!({
                        "job": job_lease.job.id,
                        "reason": queue_reason,
                        "outcomeReason": reason,
                        "queuedBy": job_lease.job.actor,
                        "queuedAt": job_lease.job.created_at.to_rfc3339(),
                        "agent": agent_id,
                        "agentName": agent_name,
                        "agentVersion": agent_version,
                        "drawerTarget": drawer_target,
                        "leaseId": lease_id,
                        "leasedAt": leased_at,
                        "reportedAt": now.to_rfc3339(),
                        "cupsJob": cups_job,
                        "physicalAction": physical_action,
                        "physicalOpenConfirmed": physical_open_confirmed,
                    });

                    let event = NewEvent {
                        dedupe: None,
                        actor: &actor,
                        kind: &outcome,
                        reference: &job_lease.job.reference,
                        detail: &event_detail,
                        amounts: &[],
                        at: now,
                    };
                    device_repo.insert_event(tx, &branch, &event).await?;

                    Ok(ReportOutcome::Recorded)
                })
            })
            .await
    }
}
