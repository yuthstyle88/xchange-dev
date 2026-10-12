//! Today's `ExchangeFacts` (design `executive-finance.md` §3, D-3). It keeps its §5 name, but reads TX-2's
//! `transaction_lines` and `transaction_payments` of COMPLETED, LIVE transactions only (lane controller's ruling on
//! #1071, issuecomment-6107626913; TX-8 #676 §3). The header fields come from `transactions`; its wide figure
//! columns (`type`, `code`, `amount`, `principal`, `fee`, `bank_fee`, `method`, …) are never read.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::ids::BranchId;

use crate::application::executive::ports::{
    BoxFuture, BranchDayTotals, ExchangeFacts, ExecutiveFilter, MethodDayTotals, SideVolumes,
    Volumes,
};

pub struct WideTransactionFacts {
    pool: PgPool,
}

impl WideTransactionFacts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// The completed LIVE transactions `t` of the scope (`$1`), on business dates `$2`..`$3` (absent: today in the
/// branch `b`'s timezone), with a line `l` of currency `$4` when set, and of the filter's channel.
fn completed_in_period(filter: &ExecutiveFilter) -> String {
    let channel = filter
        .channel
        .map(|c| format!(" AND {}", c.condition("t")))
        .unwrap_or_default();
    format!(
        "transactions t \
         JOIN branches b ON b.organization_id = t.organization_id AND b.id = t.branch_id \
         JOIN transaction_lines l ON l.transaction_id = t.id \
         WHERE t.status = 'COMPLETED' AND t.mode = 'LIVE' AND t.branch_id = ANY($1) \
           AND t.value_date >= coalesce($2::date, (now() AT TIME ZONE b.timezone)::date) \
           AND t.value_date <= coalesce($3::date, (now() AT TIME ZONE b.timezone)::date) \
           AND ($4::text IS NULL OR l.currency = $4::text){channel}"
    )
}

fn reversed_in_period(filter: &ExecutiveFilter) -> String {
    let channel = filter
        .channel
        .map(|c| format!(" AND {}", c.condition("t")))
        .unwrap_or_default();
    format!(
        "reversals r \
         JOIN branches b ON b.organization_id = r.organization_id AND b.id = r.branch_id \
         JOIN transactions t ON t.id = r.original_transaction_id \
         JOIN transaction_lines l ON l.transaction_id = t.id \
         WHERE r.status = 'POSTED' AND t.mode = 'LIVE' AND r.branch_id = ANY($1) \
           AND (r.posted_at AT TIME ZONE b.timezone)::date >= coalesce($2::date, (now() AT TIME ZONE b.timezone)::date) \
           AND (r.posted_at AT TIME ZONE b.timezone)::date <= coalesce($3::date, (now() AT TIME ZONE b.timezone)::date) \
           AND ($4::text IS NULL OR l.currency = $4::text){channel}"
    )
}

fn ids(scope: &[BranchId]) -> Vec<String> {
    scope.iter().map(|b| b.as_str().to_owned()).collect()
}

impl ExchangeFacts for WideTransactionFacts {
    fn volumes<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Volumes>> {
        Box::pin(async move {
            let sql = format!(
                "SELECT t.branch_id, l.shop_side, sum(l.thb_amount) FROM {} GROUP BY t.branch_id, l.shop_side",
                completed_in_period(filter)
            );
            let rows: Vec<(String, String, Decimal)> = sqlx::query_as(AssertSqlSafe(sql))
                .bind(ids(scope))
                .bind(filter.from)
                .bind(filter.to)
                .bind(filter.currency.as_ref().map(|c| c.to_string()))
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;
            let mut v = Volumes::default();
            let mut by_branch: BTreeMap<BranchId, SideVolumes> = BTreeMap::new();
            for (branch, side, thb) in rows {
                let row = by_branch.entry(BranchId::new(branch)).or_default();
                if side == "BUY" {
                    row.buy_thb += thb;
                    v.buy_thb += thb;
                } else {
                    row.sell_thb += thb;
                    v.sell_thb += thb;
                }
            }
            v.by_branch = by_branch;

            let rev_sql = format!(
                "SELECT l.shop_side, coalesce(sum(r.amount_thb), 0) FROM {} GROUP BY l.shop_side",
                reversed_in_period(filter)
            );
            let rev_rows: Vec<(String, Decimal)> = sqlx::query_as(AssertSqlSafe(rev_sql))
                .bind(ids(scope))
                .bind(filter.from)
                .bind(filter.to)
                .bind(filter.currency.as_ref().map(|c| c.to_string()))
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;
            for (side, thb) in rev_rows {
                if side == "BUY" {
                    v.buy_reversed_thb += thb;
                } else {
                    v.sell_reversed_thb += thb;
                }
            }

            Ok(v)
        })
    }

    fn repeat_customers<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<i64>> {
        Box::pin(async move {
            let sql = format!(
                "SELECT count(DISTINCT t.customer_id) FROM {} \
                 AND EXISTS (SELECT 1 FROM transactions e \
                   WHERE e.organization_id = t.organization_id AND e.customer_id = t.customer_id \
                     AND e.status = 'COMPLETED' AND e.mode = 'LIVE' \
                     AND (e.value_date, e.recorded_at) < (t.value_date, t.recorded_at))",
                completed_in_period(filter)
            );
            sqlx::query_scalar(AssertSqlSafe(sql))
                .bind(ids(scope))
                .bind(filter.from)
                .bind(filter.to)
                .bind(filter.currency.as_ref().map(|c| c.to_string()))
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)
        })
    }

    fn branch_day_totals<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<BranchDayTotals>> {
        Box::pin(async move {
            // A transaction's line totals, and its count, go to its first payment's method; every payment's bank fee to
            // its own (FN-1 #1068, ruling Q1).
            let rows: Vec<(String, Decimal, Decimal, Decimal, Decimal, i64)> = sqlx::query_as(
                "WITH tx AS ( \
                   SELECT t.id FROM transactions t \
                    WHERE t.branch_id = $1 AND t.value_date = $2 \
                      AND t.status = 'COMPLETED' AND t.mode = 'LIVE' \
                 ), lines AS ( \
                   SELECT l.transaction_id, \
                          coalesce(sum(l.thb_amount) FILTER (WHERE l.shop_side = 'BUY'), 0) AS buy, \
                          coalesce(sum(l.thb_amount) FILTER (WHERE l.shop_side = 'SELL'), 0) AS sell, \
                          sum(l.fee_thb) AS fee \
                     FROM transaction_lines l JOIN tx ON tx.id = l.transaction_id \
                    GROUP BY l.transaction_id \
                 ) \
                 SELECT m.method, sum(m.buy), sum(m.sell), sum(m.fee), sum(m.bank), sum(m.n)::bigint FROM ( \
                   SELECT p.method, lt.buy, lt.sell, lt.fee, 0::numeric AS bank, 1 AS n \
                     FROM lines lt \
                     JOIN transaction_payments p ON p.transaction_id = lt.transaction_id AND p.payment_no = 1 \
                   UNION ALL \
                   SELECT p.method, 0, 0, 0, p.bank_fee_thb, 0 \
                     FROM transaction_payments p JOIN tx ON tx.id = p.transaction_id \
                 ) m \
                 GROUP BY m.method ORDER BY m.method",
            )
            .bind(branch.as_str())
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
            // posted reversals by their posting business date, apart (design §3; RV-1 #682)
            let reversals_thb: Decimal = sqlx::query_scalar(
                "SELECT coalesce(sum(r.amount_thb), 0) FROM reversals r \
                   JOIN branches b ON b.organization_id = r.organization_id AND b.id = r.branch_id \
                  WHERE r.branch_id = $1 AND r.status = 'POSTED' \
                    AND (r.posted_at AT TIME ZONE b.timezone)::date = $2",
            )
            .bind(branch.as_str())
            .bind(date)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
            Ok(BranchDayTotals {
                by_method: rows
                    .into_iter()
                    .map(|(method, buy, sell, fees, bank, count)| MethodDayTotals {
                        method,
                        buy_thb: buy,
                        sell_thb: sell,
                        fees_thb: fees,
                        bank_fees_thb: bank,
                        count,
                    })
                    .collect(),
                reversals_thb,
            })
        })
    }

    fn unfinished<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<i64>> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT count(*) FROM transactions \
                  WHERE branch_id = $1 AND mode = 'LIVE' \
                    AND status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED')",
            )
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)
        })
    }
}
