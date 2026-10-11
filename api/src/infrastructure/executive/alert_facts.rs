//! Today's `AlertFacts` (design `executive-finance.md` §4.3; EX-12 folded into #1071):
//! - `ratePending`: a PENDING `RATE_PUBLICATION` approval request of the branch;
//! - `staleData`: the branch's oldest unfinished LIVE transaction is older than its `stale_data_minutes`. No
//!   `branch_alert_settings` row (or no minutes) means no alert (E-3). The design's outbox half has no reader:
//!   `outbox_events` carries no branch (ruling on #1071 gap 5);
//! - `countDifference`: a PENDING `COUNT_DIFFERENCE` approval request, or an open cash exception read through the
//!   custody reader `exception_from_row`. A row that reader cannot decode (the new cash flow's shape, until
//!   CA-10 #626) still counts as one open exception, and its detail is the row's id only: nothing here parses
//!   `data` (ruling on #1071 gap 4).

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::infrastructure::custody::decode::exception_from_row;

use crate::application::executive::drilldown::Drilldown;
use crate::application::executive::ports::{Alert, AlertFacts, AlertKind, BoxFuture};

pub struct PgAlertFacts {
    pool: PgPool,
}

impl PgAlertFacts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn rate_pending(&self, scope: &[String]) -> AppResult<Vec<Alert>> {
        let rows: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id::text, branch_id, requested_at FROM approval_requests \
              WHERE type = 'RATE_PUBLICATION' AND status = 'PENDING' AND branch_id = ANY($1) \
              ORDER BY branch_id, requested_at",
        )
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(rows
            .into_iter()
            .map(|(id, branch, at)| Alert {
                kind: AlertKind::RatePending,
                drilldown: Some(Drilldown::to(
                    "approval_requests",
                    vec![
                        ("branch", branch.clone()),
                        ("type", "RATE_PUBLICATION".to_owned()),
                        ("status", "PENDING".to_owned()),
                    ],
                )),
                branch: BranchId::new(branch),
                since: Some(at),
                detail: format!("RATE_PUBLICATION {id}"),
            })
            .collect())
    }

    async fn stale_data(&self, scope: &[String]) -> AppResult<Vec<Alert>> {
        let rows: Vec<(String, i32, DateTime<Utc>)> = sqlx::query_as(
            "SELECT s.branch_id, s.stale_data_minutes, min(t.recorded_at) \
               FROM branch_alert_settings s \
               JOIN transactions t ON t.organization_id = s.organization_id AND t.branch_id = s.branch_id \
              WHERE s.branch_id = ANY($1) AND s.stale_data_minutes IS NOT NULL \
                AND t.mode = 'LIVE' AND t.status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED') \
              GROUP BY s.branch_id, s.stale_data_minutes \
             HAVING min(t.recorded_at) < now() - make_interval(mins => s.stale_data_minutes) \
              ORDER BY s.branch_id",
        )
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(rows
            .into_iter()
            .map(|(branch, minutes, oldest)| Alert {
                kind: AlertKind::StaleData,
                drilldown: Some(Drilldown::to(
                    "admin_transactions",
                    vec![("branch", branch.clone())],
                )),
                branch: BranchId::new(branch),
                since: Some(oldest),
                detail: format!("unfinished transaction older than {minutes} minutes"),
            })
            .collect())
    }

    async fn count_difference(&self, scope: &[String]) -> AppResult<Vec<Alert>> {
        let mut alerts = Vec::new();
        let requests: Vec<(String, String, DateTime<Utc>, Value)> = sqlx::query_as(
            "SELECT id::text, branch_id, requested_at, proposed_payload FROM approval_requests \
              WHERE type = 'COUNT_DIFFERENCE' AND status = 'PENDING' AND branch_id = ANY($1) \
              ORDER BY branch_id, requested_at",
        )
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        for (id, branch, at, payload) in requests {
            let named = |k: &str| payload.get(k).and_then(Value::as_str).map(str::to_owned);
            let detail = match (named("currency"), named("difference")) {
                (Some(c), Some(d)) => format!("COUNT_DIFFERENCE {id}: {c} {d}"),
                _ => format!("COUNT_DIFFERENCE {id}"),
            };
            alerts.push(count_alert(branch, Some(at), detail));
        }

        let rows = sqlx::query(
            "SELECT id, data, branch_id FROM cash_exceptions \
              WHERE branch_id = ANY($1) ORDER BY branch_id, position",
        )
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        for row in &rows {
            let branch: String = sqlx::Row::get(row, 2);
            match exception_from_row(row) {
                Ok(e) if e.status == ExceptionStatus::Open => {
                    let since = Utc.timestamp_millis_opt(e.at.epoch_millis()).single();
                    alerts.push(count_alert(branch, since, exception_detail(&e)));
                }
                Ok(_) => {}
                Err(_) => {
                    let id: String = sqlx::Row::get(row, 0);
                    alerts.push(count_alert(branch, None, id));
                }
            }
        }
        Ok(alerts)
    }
}

/// Neither `cash_counts` (CA-8 #624) nor `finance_reconciliation` (FN-1 #1068) is on `main`: no drill-down.
fn count_alert(branch: String, since: Option<DateTime<Utc>>, detail: String) -> Alert {
    Alert {
        kind: AlertKind::CountDifference,
        branch: BranchId::new(branch),
        since,
        detail,
        drilldown: None,
    }
}

/// The exception's id and the currencies and differences it holds.
fn exception_detail(e: &CashException) -> String {
    let mut parts = vec![e.id.clone()];
    for delta in [&e.target_difference, &e.cash_variance, &e.safe_variance]
        .into_iter()
        .flatten()
    {
        for (currency, d) in delta.non_zero() {
            parts.push(format!(
                "{currency} {}",
                Decimal::from_i128_with_scale(d.minor(), 2)
            ));
        }
    }
    parts.join(": ")
}

impl AlertFacts for PgAlertFacts {
    fn alerts<'a>(&'a self, scope: &'a [BranchId]) -> BoxFuture<'a, AppResult<Vec<Alert>>> {
        Box::pin(async move {
            let ids: Vec<String> = scope.iter().map(|b| b.as_str().to_owned()).collect();
            let mut alerts = self.rate_pending(&ids).await?;
            alerts.extend(self.stale_data(&ids).await?);
            alerts.extend(self.count_difference(&ids).await?);
            Ok(alerts)
        })
    }
}
