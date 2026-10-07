//! The loop: claim, send, record (§5.4c *Claim → send → record*, *Restart and offline*).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;

use crate::client::{AgentApi, ApiError, ClaimRequest, DrawerInfo, Outcome, Report};
pub use crate::clock::{Clock, FakeClock, ManualClock, SystemClock};
use crate::journal::{Journal, Pending};
use crate::transport::DrawerTransport;

static JITTER_COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_jitter() -> f64 {
    let count = JITTER_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    let seed = nanos ^ (count.wrapping_mul(0x517cc1b727220a95));
    let fraction = (seed % 1_000_000) as f64 / 1_000_000.0;
    fraction * 2.0 - 1.0
}

/// What the agent says about itself at every claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub version: String,
    pub build: String,
    pub os: String,
    pub host: String,
    pub drawer: DrawerInfo,
    /// The long-poll asked for; at most 20,000.
    pub wait_ms: u64,
}

/// What one step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The claim answered 204.
    Idle,
    /// A job was claimed and its outcome recorded (reported or still owed).
    Done(Outcome),
    /// 401: stop claiming; log *re-pair needed*; check again every 10 minutes.
    RePairNeeded,
    /// No answer, or 429: wait this long before the next claim.
    Wait(Duration),
}

pub struct AgentRunner<A, T, C = SystemClock> {
    pub api: A,
    pub transport: T,
    pub journal: Journal,
    pub config: AgentConfig,
    /// Consecutive failed calls, for [`backoff`].
    pub failures: u32,
    pub clock: C,
}

impl<A: AgentApi, T: DrawerTransport> AgentRunner<A, T, SystemClock> {
    pub fn new(api: A, transport: T, journal: Journal, config: AgentConfig) -> Self {
        Self {
            api,
            transport,
            journal,
            config,
            failures: 0,
            clock: SystemClock,
        }
    }
}

impl<A: AgentApi, T: DrawerTransport, C: Clock> AgentRunner<A, T, C> {
    pub fn with_clock(
        api: A,
        transport: T,
        journal: Journal,
        config: AgentConfig,
        clock: C,
    ) -> Self {
        Self {
            api,
            transport,
            journal,
            config,
            failures: 0,
            clock,
        }
    }

    pub fn new_with_clock(
        api: A,
        transport: T,
        journal: Journal,
        config: AgentConfig,
        clock: C,
    ) -> Self {
        Self::with_clock(api, transport, journal, config, clock)
    }

    /// The claim this runner sends.
    pub fn claim_request(&self) -> ClaimRequest {
        ClaimRequest {
            wait_ms: self.config.wait_ms,
            version: self.config.version.clone(),
            build: self.config.build.clone(),
            os: self.config.os.clone(),
            host: self.config.host.clone(),
            drawer: self.config.drawer.clone(),
        }
    }

    /// At start: a send with no outcome is reported `FAILED` `AGENT_RESTARTED` and never sent
    /// again; an unacknowledged outcome is reported again. What the server takes is
    /// acknowledged in the journal; what it does not stays owed.
    /// Retries reporting owed items from the journal.
    /// Only Connection errors and 429 keep a report owed.
    /// A Rejected answer is final: it is acknowledged so it is no longer owed.
    /// A still-owed report must not stop the agent from claiming new jobs.
    async fn retry_owed(&mut self) -> Result<(), ApiError> {
        let owed = self.journal.pending(SystemTime::now());
        for item in owed {
            match item {
                Pending::Sending { job_id, lease_id } => {
                    let report = Report {
                        job_id: job_id.clone(),
                        lease_id: lease_id.clone(),
                        outcome: Outcome::Failed,
                        reason: "AGENT_RESTARTED".to_owned(),
                        detail: serde_json::json!({}),
                    };
                    let _ = self.journal.outcome(&report, SystemTime::now());
                    match self.api.report(&report).await {
                        Ok(()) => {
                            let _ = self.journal.acknowledged(&job_id, &lease_id);
                        }
                        Err(ApiError::Rejected { status, error }) => {
                            eprintln!("report rejected ({status}): {error}");
                            let _ = self.journal.rejected(&job_id, &lease_id, status, &error);
                        }
                        Err(ApiError::Unauthorized) => {
                            return Err(ApiError::Unauthorized);
                        }
                        Err(ApiError::Connection(_)) | Err(ApiError::RateLimited { .. }) => {
                            // Only Connection and 429 keep a report owed; do not stop claiming
                        }
                    }
                }
                Pending::Unacknowledged(report) => {
                    match self.api.report(&report).await {
                        Ok(()) => {
                            let _ = self.journal.acknowledged(&report.job_id, &report.lease_id);
                        }
                        Err(ApiError::Rejected { status, error }) => {
                            eprintln!("report rejected ({status}): {error}");
                            let _ = self.journal.rejected(
                                &report.job_id,
                                &report.lease_id,
                                status,
                                &error,
                            );
                        }
                        Err(ApiError::Unauthorized) => {
                            return Err(ApiError::Unauthorized);
                        }
                        Err(ApiError::Connection(_)) | Err(ApiError::RateLimited { .. }) => {
                            // Only Connection and 429 keep a report owed; do not stop claiming
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// At start: a send with no outcome is reported `FAILED` `AGENT_RESTARTED` and never sent
    /// again; an unacknowledged outcome is reported again. What the server takes is
    /// acknowledged in the journal; what it does not stays owed.
    pub async fn recover(&mut self) {
        let _ = self.retry_owed().await;
    }

    /// One claim → send → record.
    pub async fn step(&mut self) -> Step {
        // First retry any owed reports from the journal (401 stops claiming)
        if let Err(ApiError::Unauthorized) = self.retry_owed().await {
            return Step::RePairNeeded;
        }

        let req = self.claim_request();
        let claim_res = self.api.claim(&req).await;

        let claimed = match claim_res {
            Ok(None) => {
                self.failures = 0;
                return Step::Idle;
            }
            Ok(Some(claimed)) => {
                self.failures = 0;
                claimed
            }
            Err(ApiError::Unauthorized) => {
                return Step::RePairNeeded;
            }
            Err(ApiError::RateLimited { retry_after_secs }) => {
                self.failures += 1;
                let wait = backoff(self.failures - 1, random_jitter())
                    .max(Duration::from_secs(retry_after_secs));
                return Step::Wait(wait);
            }
            Err(ApiError::Rejected { .. }) | Err(ApiError::Connection(_)) => {
                self.failures += 1;
                let wait = backoff(self.failures - 1, random_jitter());
                return Step::Wait(wait);
            }
        };

        // Note arrival on monotonic clock
        let arrival = self.clock.now();

        // Decode kick bytes
        let (outcome, reason, detail) =
            match base64::engine::general_purpose::STANDARD.decode(&claimed.bytes) {
                Err(_) => (
                    Outcome::Failed,
                    "BAD_BYTES".to_owned(),
                    serde_json::json!({}),
                ),
                Ok(bytes) => {
                    // Write sending to journal before sending bytes out
                    if self
                        .journal
                        .sending(&claimed.job_id, &claimed.lease_id, SystemTime::now())
                        .is_err()
                    {
                        (
                            Outcome::Failed,
                            "JOURNAL_WRITE_FAILED".to_owned(),
                            serde_json::json!({}),
                        )
                    } else {
                        // Check if send window has already passed
                        let elapsed = self.clock.now().saturating_duration_since(arrival);
                        if elapsed >= Duration::from_millis(claimed.send_within_ms) {
                            (
                                Outcome::Expired,
                                "SEND_WINDOW_PASSED".to_owned(),
                                serde_json::json!({}),
                            )
                        } else {
                            // Send within 5s
                            match tokio::time::timeout(
                                Duration::from_secs(5),
                                self.transport.send(&bytes),
                            )
                            .await
                            {
                                Ok(Ok(receipt)) => {
                                    let detail = match receipt.cups_job {
                                        Some(job) => serde_json::json!({ "cupsJob": job }),
                                        None => serde_json::json!({}),
                                    };
                                    (Outcome::DrawerOpened, String::new(), detail)
                                }
                                Ok(Err(err)) => (
                                    Outcome::Failed,
                                    err.reason().to_owned(),
                                    serde_json::json!({}),
                                ),
                                Err(_) => (
                                    Outcome::Failed,
                                    "SEND_FAILED".to_owned(),
                                    serde_json::json!({}),
                                ),
                            }
                        }
                    }
                }
            };

        // Write outcome to journal
        let report = Report {
            job_id: claimed.job_id.clone(),
            lease_id: claimed.lease_id.clone(),
            outcome,
            reason,
            detail,
        };
        let _ = self.journal.outcome(&report, SystemTime::now());

        // Report to server; acknowledge if taken or rejected
        match self.api.report(&report).await {
            Ok(()) => {
                let _ = self
                    .journal
                    .acknowledged(&claimed.job_id, &claimed.lease_id);
            }
            Err(ApiError::Rejected { status, error }) => {
                eprintln!("report rejected ({status}): {error}");
                let _ = self
                    .journal
                    .rejected(&claimed.job_id, &claimed.lease_id, status, &error);
            }
            Err(_) => {
                // Connection or RateLimited: stays owed!
            }
        }

        Step::Done(outcome)
    }
}

/// The wait after `attempt` consecutive failures (0 = the first): 1 s doubling up to 60 s,
/// then scaled by `1 + 0.2 × jitter`, `jitter` in `-1.0..=1.0`.
pub fn backoff(attempt: u32, jitter: f64) -> Duration {
    let base_secs = if attempt >= 6 {
        60.0
    } else {
        (1u64 << attempt) as f64
    };
    let scaled = base_secs * (1.0 + 0.2 * jitter);
    Duration::from_secs_f64(scaled.max(0.0))
}
