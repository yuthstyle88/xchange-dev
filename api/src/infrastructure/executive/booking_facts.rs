//! `BookingFacts`: booking conversion facts read from the Blueprint bookings tables (design `executive-finance.md` §3).
//! EX-8 (#1190).

use std::collections::BTreeMap;

use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::application::executive::metrics::MetricUnavailable;
use crate::application::executive::ports::{
    BookingConversion, BoxFuture, BranchDayTotals, ExchangeFacts, ExecutiveFilter, Volumes,
};
use crate::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;

pub struct BookingFacts {
    pool: PgPool,
    wide: WideTransactionFacts,
}

impl BookingFacts {
    pub fn new(pool: PgPool) -> Self {
        Self {
            wide: WideTransactionFacts::new(pool.clone()),
            pool,
        }
    }
}

impl ExchangeFacts for BookingFacts {
    fn volumes<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Volumes>> {
        self.wide.volumes(scope, filter)
    }

    fn repeat_customers<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<i64>> {
        self.wide.repeat_customers(scope, filter)
    }

    fn branch_day_totals<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<BranchDayTotals>> {
        self.wide.branch_day_totals(branch, date)
    }

    fn unfinished<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<i64>> {
        self.wide.unfinished(branch)
    }

    fn unfinished_by_branch<'a>(
        &'a self,
        branches: &'a [BranchId],
    ) -> BoxFuture<'a, AppResult<BTreeMap<BranchId, i64>>> {
        self.wide.unfinished_by_branch(branches)
    }

    /// The booking conversion over `branches` and the filter (design §3).
    ///
    /// Cohort: bookings whose slot `service_date` falls in the period and is not after the branch's today.
    /// - `numerator` (completed): cohort bookings now `COMPLETED`
    /// - `denominator` (confirmed): cohort bookings that reached `CONFIRMED`
    fn booking_conversion<'a>(
        &'a self,
        branches: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<BookingConversion, MetricUnavailable>>> {
        let branch_ids: Vec<String> = branches.iter().map(|b| b.as_str().to_owned()).collect();
        let currency = filter.currency.as_ref().map(|c| c.as_str().to_owned());
        let from = filter.from;
        let to = filter.to;

        Box::pin(async move {
            let mut by_branch: BTreeMap<BranchId, (i64, i64)> =
                branches.iter().cloned().map(|b| (b, (0, 0))).collect();

            if branches.is_empty() {
                return Ok(Ok(BookingConversion::new((0, 0), by_branch)));
            }

            let rows: Vec<(String, i64, i64)> = sqlx::query_as(
                "SELECT
                    b.branch_id,
                    count(*) FILTER (WHERE b.status = 'COMPLETED') AS completed,
                    count(*) FILTER (
                        WHERE b.status IN ('CONFIRMED', 'CHECKED_IN', 'IN_SERVICE', 'COMPLETED')
                           OR EXISTS (
                               SELECT 1 FROM booking_events e
                               WHERE e.booking_id = b.id AND e.to_status = 'CONFIRMED'
                           )
                    ) AS confirmed
                 FROM bookings b
                 JOIN booking_slots s ON s.id = b.slot_id AND s.organization_id = b.organization_id AND s.branch_id = b.branch_id
                 JOIN branches br ON br.organization_id = b.organization_id AND br.id = b.branch_id
                 JOIN customers c ON c.organization_id = b.organization_id AND c.id = b.customer_id
                 WHERE b.branch_id = ANY($1)
                   AND c.mode = 'LIVE'
                   AND s.service_date >= coalesce($2::date, (now() AT TIME ZONE br.timezone)::date)
                   AND s.service_date <= coalesce($3::date, (now() AT TIME ZONE br.timezone)::date)
                   AND s.service_date <= (now() AT TIME ZONE br.timezone)::date
                   AND ($4::text IS NULL OR b.requested_currency = $4::text)
                 GROUP BY b.branch_id",
            )
            .bind(&branch_ids)
            .bind(from)
            .bind(to)
            .bind(currency)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            for (branch_id, completed, confirmed) in rows {
                let id = BranchId::new(branch_id);
                if by_branch.contains_key(&id) {
                    by_branch.insert(id, (completed, confirmed));
                }
            }

            let total = by_branch.values().fold((0i64, 0i64), |acc, &(comp, conf)| {
                (acc.0 + comp, acc.1 + conf)
            });

            Ok(Ok(BookingConversion::new(total, by_branch)))
        })
    }
}
