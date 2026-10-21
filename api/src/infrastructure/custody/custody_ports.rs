//! `PgCustodyPorts`: the custody service's questions to the other contexts.
//!
//! `xc-shift` may not depend on the crates that own rates, exchanges, customers,
//! files and employees, so it declares what it needs to know as the trait
//! `CustodyPorts`. This is its implementation, in the root package because the
//! root package is the one place allowed to know every context. It only reads,
//! and always through the transaction it is handed, so the custody service sees
//! the same state it is about to change.
//!
//! The rules are the old server's custody code, which read them off its admin
//! snapshot.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Offset, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::branch_hours::{BranchHours, HoursWindow};
use xc_rate::domain::validation::format_iso;
use xc_shift::application::custody::service::CustodyFactsPort;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;

/// The trait's future: `futures::future::BoxFuture` spelled out.
type Answer<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

/// Answers `CustodyPorts` from the tables of the other contexts.
#[derive(Debug, Clone, Copy, Default)]
pub struct PgCustodyPorts;

impl CustodyPorts for PgCustodyPorts {
    /// - `has_live_rate`: the branch has a current publication (its counter
    ///   rates, rates.md D-3);
    /// - `rates_updated`: the `published_at` of the branch's current publication
    ///   (#604 ruling: no `rates` rows since RE-7), printed with milliseconds and
    ///   `Z`; empty when the branch has none;
    /// - `pending_transactions`: the ids of the shift's unfinished exchanges
    ///   (`DRAFT`, `PENDING_APPROVAL`, `CONFIRMED`), oldest first;
    /// - `last_transaction`: the document number of the last COMPLETED live exchange of
    ///   `open_shift_id` in this branch.
    fn outside_facts<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        open_shift_id: Option<&'a str>,
    ) -> Answer<'a, OutsideFacts> {
        Box::pin(async move {
            let published_at: Option<DateTime<Utc>> = sqlx::query_scalar(
                "SELECT p.published_at FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 WHERE b.id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let (last_transaction, pending_transactions) = match open_shift_id {
                Some(shift_id) => {
                    let last: Option<String> = sqlx::query_scalar(
                        "SELECT document_ref FROM transactions \
                         WHERE branch_id = $1 AND shift_id = $2 AND status = 'COMPLETED' \
                         ORDER BY seq DESC LIMIT 1",
                    )
                    .bind(branch.as_str())
                    .bind(shift_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    let pending: Vec<String> = sqlx::query_scalar(
                        "SELECT id::text FROM transactions \
                         WHERE branch_id = $1 AND shift_id = $2 \
                           AND status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED') \
                         ORDER BY seq ASC",
                    )
                    .bind(branch.as_str())
                    .bind(shift_id)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    (last, pending)
                }
                None => (None, Vec::new()),
            };

            Ok(OutsideFacts {
                has_live_rate: published_at.is_some(),
                rates_updated: published_at.map(format_iso).unwrap_or_default(),
                pending_transactions,
                last_transaction,
            })
        })
    }

    /// A live customer of this branch's organisation with that id.
    fn customer_known<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> Answer<'a, bool> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT EXISTS ( \
                     SELECT 1 FROM customers c \
                     JOIN branches b ON b.organization_id = c.organization_id \
                     WHERE b.id = $1 AND c.id = $2 AND c.mode = 'LIVE' \
                 )",
            )
            .bind(branch.as_str())
            .bind(customer_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)
        })
    }

    /// Every key is a file this owner stored in this branch. No key at all is
    /// owned: how many are needed is the service's rule.
    fn evidence_owned<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        owner_id: &'a str,
        keys: &'a [String],
    ) -> Answer<'a, bool> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT NOT EXISTS (SELECT 1 FROM unnest($3::text[]) AS wanted (key) \
                 WHERE NOT EXISTS (SELECT 1 FROM files f \
                 WHERE f.key = wanted.key AND f.owner_id = $2 AND f.branch_id = $1))",
            )
            .bind(branch.as_str())
            .bind(owner_id)
            .bind(keys)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)
        })
    }

    fn service_window<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: CustodyTime,
    ) -> Answer<'a, ServiceWindow> {
        Box::pin(async move {
            let row: Option<(String, Option<serde_json::Value>)> =
                sqlx::query_as("SELECT timezone, opening_hours FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

            let (timezone, opening_hours) = match row {
                Some((tz, Some(hours))) if !hours.is_null() => (tz, hours),
                _ => return Ok(ServiceWindow::NotSet),
            };

            let hours = BranchHours::new(&timezone, Some(&opening_hours))?;

            let now_millis = now.epoch_millis();
            let now_utc = match DateTime::<Utc>::from_timestamp_millis(now_millis) {
                Some(dt) => dt,
                None => return Ok(ServiceWindow::Closed),
            };

            let tz: Tz = match timezone.parse() {
                Ok(t) => t,
                Err(_) => return Ok(ServiceWindow::Closed),
            };

            let now_local = now_utc.with_timezone(&tz);
            let local_today = now_local.date_naive();

            for offset in [-2, -1, 0, 1] {
                if let Some(date) = local_today.checked_add_signed(chrono::Duration::days(offset))
                    && let HoursWindow::Open { open, close } = hours.window(date)
                    && open <= now_utc
                    && now_utc < close
                {
                    return Ok(ServiceWindow::Open {
                        open: CustodyTime::from_epoch_millis(open.timestamp_millis()),
                        close: CustodyTime::from_epoch_millis(close.timestamp_millis()),
                    });
                }
            }

            if let HoursWindow::Open { open, close } = hours.window(local_today) {
                return Ok(ServiceWindow::Open {
                    open: CustodyTime::from_epoch_millis(open.timestamp_millis()),
                    close: CustodyTime::from_epoch_millis(close.timestamp_millis()),
                });
            }

            Ok(ServiceWindow::Closed)
        })
    }

    fn utc_offset_minutes<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        at: CustodyTime,
    ) -> Answer<'a, i32> {
        Box::pin(async move {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT timezone FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

            let timezone = match row {
                Some((tz,)) => tz,
                None => return Ok(0),
            };

            let tz: Tz = match timezone.parse() {
                Ok(t) => t,
                Err(_) => return Ok(0),
            };

            let at_millis = at.epoch_millis();
            let at_utc = match DateTime::<Utc>::from_timestamp_millis(at_millis) {
                Some(dt) => dt,
                None => return Ok(0),
            };

            let at_local = at_utc.with_timezone(&tz);
            let offset_secs = at_local.offset().fix().local_minus_utc();
            Ok(offset_secs / 60)
        })
    }
}

impl CustodyFactsPort for PgCustodyPorts {
    fn service_window_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: CustodyTime,
    ) -> Answer<'a, (ServiceWindow, i32)> {
        Box::pin(async move {
            let window = self.service_window(tx, branch, now).await?;
            let offset = self.utc_offset_minutes(tx, branch, now).await?;
            Ok((window, offset))
        })
    }

    fn has_live_rate_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> Answer<'a, (bool, String)> {
        Box::pin(async move {
            let facts = self.outside_facts(tx, branch, None).await?;
            Ok((facts.has_live_rate, facts.rates_updated))
        })
    }

    fn transactions_summary_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> Answer<'a, (Vec<String>, Option<String>)> {
        Box::pin(async move {
            let facts = self.outside_facts(tx, branch, None).await?;
            Ok((facts.pending_transactions, facts.last_transaction))
        })
    }

    fn customer_exists_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> Answer<'a, bool> {
        self.customer_known(tx, branch, customer_id)
    }
}
