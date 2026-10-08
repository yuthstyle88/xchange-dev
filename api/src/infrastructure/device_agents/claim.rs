//! The claim block of the agent service, filled by row AG-2 (design §5.4c *Claim*).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use super::agent_repository::{AgentRepository, AgentSeen, ClaimArgs, DeviceAgent, DrawerTarget};
use super::agent_service::AgentService;
use crate::infrastructure::devices::device_job::JobStatus;
use crate::infrastructure::devices::device_repository::{DeviceRepository, NewEvent};
use crate::infrastructure::devices::printer_models::DrawerSettings;

const STANDARD_BASE64: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Hand-written standard base64 encoder with padding (~15 lines, no crate).
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(STANDARD_BASE64[((n >> 18) & 63) as usize] as char);
        out.push(STANDARD_BASE64[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(STANDARD_BASE64[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(STANDARD_BASE64[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Outcome of a claim call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Leased {
        job_id: String,
        lease_id: String,
        bytes: String,
        send_within_ms: i64,
    },
    NoContent,
    RateLimited {
        retry_after: u64,
    },
    BranchMismatch,
    Revoked,
}

/// Parameters for claiming a drawer job.
#[derive(Debug, Clone)]
pub struct ClaimParams<'a> {
    pub wait_ms: u64,
    pub version: &'a str,
    pub build: &'a str,
    pub os: &'a str,
    pub host: &'a str,
    pub drawer_transport: &'a str,
    pub drawer_target: &'a str,
    pub branch_id: Option<&'a str>,
}

impl AgentService {
    /// Claims the next drawer job for an agent or waits up to `wait_ms`.
    pub async fn claim<F>(
        &self,
        agent: &DeviceAgent,
        params: &ClaimParams<'_>,
        now_fn: F,
    ) -> AppResult<ClaimOutcome>
    where
        F: Fn() -> DateTime<Utc> + Send + Sync,
    {
        let now = now_fn();

        // 1. Branch mismatch check
        if let Some(named_branch) = params.branch_id
            && named_branch != agent.branch_id
        {
            let branch = BranchId::new(&agent.branch_id);
            let agent_id = agent.id.clone();
            let named_branch = named_branch.to_string();
            let device_repo = Arc::clone(&self.device_repo);
            let b = branch.clone();
            self.branch_tx
                .run(&b, move |tx| {
                    let branch = branch.clone();
                    let device_repo = Arc::clone(&device_repo);
                    let actor = format!("agent:{agent_id}");
                    let detail = serde_json::json!({
                        "namedBranch": named_branch,
                    });
                    Box::pin(async move {
                        let event = NewEvent {
                            dedupe: None,
                            actor: &actor,
                            kind: "AGENT_BRANCH_MISMATCH",
                            reference: "",
                            detail: &detail,
                            amounts: &[],
                            at: now,
                        };
                        device_repo.insert_event(tx, &branch, &event).await?;
                        Ok(())
                    })
                })
                .await?;
            return Ok(ClaimOutcome::BranchMismatch);
        }

        // 2. Touch and rate limit
        let seen = AgentSeen {
            version: params.version,
            build: params.build,
            os: params.os,
            host: params.host,
            drawer: Some(DrawerTarget {
                transport: params.drawer_transport,
                target: params.drawer_target,
            }),
        };
        let Some(window) = self.agent_repo.touch(&agent.id, &seen, now).await? else {
            return Ok(ClaimOutcome::Revoked);
        };

        if window.count > 60 {
            let retry_after = (window.window_start + chrono::Duration::seconds(60) - now)
                .num_seconds()
                .max(1) as u64;
            return Ok(ClaimOutcome::RateLimited { retry_after });
        }

        // 3. Probe and long poll (holding no lock/tx)
        let branch = BranchId::new(&agent.branch_id);
        let max_wait = std::time::Duration::from_millis(params.wait_ms.min(20_000));
        let start = std::time::Instant::now();
        let mut current_now = now;

        let mut claimable = self.agent_repo.has_claimable(&branch, current_now).await?;
        if !claimable && !max_wait.is_zero() {
            while start.elapsed() < max_wait {
                let remaining = max_wait.saturating_sub(start.elapsed());
                let sleep_step = remaining.min(std::time::Duration::from_millis(500));
                if sleep_step.is_zero() {
                    break;
                }
                tokio::time::sleep(sleep_step).await;
                current_now = now_fn();
                if self.agent_repo.has_claimable(&branch, current_now).await? {
                    claimable = true;
                    break;
                }
            }
        }

        if !claimable {
            return Ok(ClaimOutcome::NoContent);
        }

        // 4. One BranchTx::run transaction
        let agent_repo = Arc::clone(&self.agent_repo);
        let device_repo = Arc::clone(&self.device_repo);
        let printer_models = self.printer_models.clone();
        let agent_id = agent.id.clone();
        let b = branch.clone();

        self.branch_tx
            .run(&b, move |tx| {
                let branch = branch.clone();
                let agent_repo = Arc::clone(&agent_repo);
                let device_repo = Arc::clone(&device_repo);
                let printer_models = printer_models.clone();
                let agent_id = agent_id.clone();

                Box::pin(async move {
                    loop {
                        let lease_id = uuid::Uuid::new_v4().to_string();
                        let stale_before = current_now - chrono::Duration::seconds(10);
                        let lease_until = current_now + chrono::Duration::seconds(10);
                        let args = ClaimArgs {
                            agent_id: &agent_id,
                            lease_id: &lease_id,
                            now: current_now,
                            stale_before,
                            lease_until,
                        };
                        let step = agent_repo.claim_in(tx, &branch, &args).await?;
                        let actor = format!("agent:{agent_id}");

                        for job in step.expired {
                            let event = NewEvent {
                                dedupe: None,
                                actor: &actor,
                                kind: "EXPIRED",
                                reference: &job.reference,
                                detail: &serde_json::json!({ "job": job.id }),
                                amounts: &[],
                                at: current_now,
                            };
                            device_repo.insert_event(tx, &branch, &event).await?;
                        }

                        for job in step.lapsed {
                            let event = NewEvent {
                                dedupe: None,
                                actor: &actor,
                                kind: "FAILED",
                                reference: &job.reference,
                                detail: &serde_json::json!({ "job": job.id, "reason": "LEASE_LAPSED" }),
                                amounts: &[],
                                at: current_now,
                            };
                            device_repo.insert_event(tx, &branch, &event).await?;
                        }

                        let Some(job) = step.leased else {
                            return Ok(ClaimOutcome::NoContent);
                        };

                        let event = NewEvent {
                            dedupe: None,
                            actor: &actor,
                            kind: "JOB_LEASED",
                            reference: &job.reference,
                            detail: &serde_json::json!({ "job": job.id, "leaseId": lease_id }),
                            amounts: &[],
                            at: current_now,
                        };
                        device_repo.insert_event(tx, &branch, &event).await?;

                        let drawer_settings = DrawerSettings::read_in(
                            tx,
                            &*device_repo,
                            &branch,
                            &printer_models,
                        )
                        .await?;
                        let pulse = printer_models
                            .get(&drawer_settings.model)
                            .and_then(|m| m.drawer_pulse);

                        let Some(pulse) = pulse else {
                            agent_repo
                                .report_in(
                                    tx,
                                    &branch,
                                    &job.id,
                                    &lease_id,
                                    JobStatus::Leased,
                                    JobStatus::Failed,
                                    Some("NO_PULSE"),
                                    current_now,
                                )
                                .await?;
                            let event = NewEvent {
                                dedupe: None,
                                actor: &actor,
                                kind: "FAILED",
                                reference: &job.reference,
                                detail: &serde_json::json!({ "job": job.id, "reason": "NO_PULSE" }),
                                amounts: &[],
                                at: current_now,
                            };
                            device_repo.insert_event(tx, &branch, &event).await?;
                            continue;
                        };

                        let kick = pulse.kick_bytes();
                        let bytes = base64_encode(&kick);
                        let send_within_ms = (job.created_at + chrono::Duration::seconds(10)
                            - current_now)
                            .num_milliseconds()
                            .max(0);

                        return Ok(ClaimOutcome::Leased {
                            job_id: job.id,
                            lease_id,
                            bytes,
                            send_within_ms,
                        });
                    }
                })
            })
            .await
    }
}
