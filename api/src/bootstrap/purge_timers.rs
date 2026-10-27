//! K6/P13: the retention purge (`RetentionJob::purge`) and the expired-session purge run on a timer, every
//! `auth.purge_interval_secs` like the verification sweeper. One spawn line in `bootstrap/mod.rs` starts `run`;
//! it stops when `Application`'s stop channel says so (or its sender is dropped), as the market refresh does.

use std::time::Duration;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;
use xc_alert::infrastructure::rate_alert_retention::PgRateAlertPurger;
use xc_app::infrastructure::planner_retention::PgPlannerDraftPurger;
use xc_auth::infrastructure::expired_sessions::PgExpiredSessions;
use xc_customer::application::retention_job::RetentionJob;

/// The retention job with every other crate's purger: `PLANNER_DRAFT` (`xc-app`) and `RATE_ALERT` (`xc-alert`).
pub fn retention_job(pool: PgPool) -> RetentionJob {
    RetentionJob::pg(pool.clone())
        .with_purger(Arc::new(PgPlannerDraftPurger::new(pool.clone())))
        .with_purger(Arc::new(PgRateAlertPurger::new(pool)))
}

/// What one round did: each purge's own result, so one failing never hides or stops the other.
#[derive(Debug)]
pub struct PurgeRound {
    pub retention: AppResult<u64>,
    pub sessions: AppResult<u64>,
}

/// One round at `now`: the retention purge, then the expired-session purge, each run even when the other fails.
pub async fn purge_round(
    retention: &RetentionJob,
    sessions: &PgExpiredSessions,
    now: DateTime<Utc>,
) -> PurgeRound {
    let _ = (retention, sessions, now);
    todo!("ticket 4: run both purges")
}

/// Runs `purge_round` at once and then every `every` with the time of that round, logging each result (a failed
/// round is logged and waited out), until `stop` holds `true` or its sender is dropped.
pub async fn run(pool: PgPool, every: Duration, stop: watch::Receiver<bool>) {
    let _ = (pool, every, stop);
    todo!("ticket 4: the timer loop")
}
