//! `LedgerCashFacts`: cash facts read from the ledger and inventory tables (design `executive-finance.md` §3).
//! EX-7 (#1076).

use pos_error::{AppError, AppResult};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

use crate::application::executive::metrics::MetricUnavailable;
use crate::application::executive::ports::{BoxFuture, CashFacts, ExecutiveFilter, ShiftStatus};

pub struct LedgerCashFacts {
    pool: PgPool,
}

impl LedgerCashFacts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CashFacts for LedgerCashFacts {
    fn available_cash<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Decimal>> {
        Box::pin(async move {
            let on_hand = self.on_hand_by_currency(branch).await?;
            // reservations: none exist before #555 phase 6
            Ok(on_hand
                .into_iter()
                .filter(|(c, _)| c.as_str() == "THB")
                .map(|(_, q)| q)
                .sum())
        })
    }

    fn in_transit<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Decimal>> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT coalesce(sum(e.quantity_signed), 0) \
                   FROM ledger_entries e \
                   JOIN cash_accounts a ON a.id = e.account_id \
                  WHERE a.branch_id = $1 \
                    AND a.currency = 'THB' \
                    AND a.kind = 'IN_TRANSIT'",
            )
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)
        })
    }

    fn shift_status<'a>(
        &'a self,
        branch: &'a BranchId,
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<ShiftStatus>> {
        Box::pin(async move {
            let (open, verified, unverified): (i64, i64, i64) = sqlx::query_as(
                "SELECT count(*) FILTER (WHERE s.status = 'OPEN'), \
                        count(*) FILTER (WHERE s.status = 'CLOSED' AND in_period AND verified), \
                        count(*) FILTER (WHERE s.status = 'CLOSED' AND in_period AND NOT verified) \
                   FROM ( \
                     SELECT s.status, \
                            s.closed_at IS NOT NULL \
                              AND (s.closed_at AT TIME ZONE b.timezone)::date \
                                  >= coalesce($2::date, (now() AT TIME ZONE b.timezone)::date) \
                              AND (s.closed_at AT TIME ZONE b.timezone)::date \
                                  <= coalesce($3::date, (now() AT TIME ZONE b.timezone)::date) AS in_period, \
                            EXISTS (SELECT 1 FROM cash_counts c \
                                     WHERE c.shift_id = s.id AND c.kind = 'CLOSE' AND c.verified_by IS NOT NULL) \
                              AS verified \
                       FROM shifts s JOIN branches b ON b.id = s.branch_id \
                      WHERE s.branch_id = $1 \
                   ) s",
            )
            .bind(branch.as_str())
            .bind(filter.from)
            .bind(filter.to)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
            Ok(ShiftStatus {
                open,
                closed_verified: verified,
                closed_unverified: unverified,
            })
        })
    }

    fn on_hand_by_currency<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<(CurrencyCode, Decimal)>>> {
        Box::pin(async move {
            let rows: Vec<(String, Decimal)> = sqlx::query_as(
                "SELECT code, sum(amount) FROM ( \
                    SELECT a.currency::text AS code, sum(e.quantity_signed) AS amount \
                      FROM ledger_entries e \
                      JOIN cash_accounts a ON a.id = e.account_id \
                     WHERE a.branch_id = $1 \
                       AND a.currency = 'THB' \
                       AND a.kind IN ('DRAWER', 'SAFE') \
                     GROUP BY a.currency \
                    UNION ALL \
                    SELECT ip.currency::text AS code, ip.quantity AS amount \
                      FROM inventory_positions ip \
                     WHERE ip.branch_id = $1 \
                       AND ip.currency <> 'THB' \
                 ) h GROUP BY code ORDER BY code",
            )
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            rows.into_iter()
                .map(|(code, q)| {
                    CurrencyCode::parse(code.trim())
                        .map(|c| (c, q))
                        .map_err(|_| AppError::Internal(format!("on hand: currency {code:?}")))
                })
                .collect()
        })
    }

    fn realized_margin<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<Money, MetricUnavailable>>> {
        Box::pin(async move {
            if scope.is_empty() {
                return Ok(Err(MetricUnavailable::AwaitsLedger));
            }

            let ids: Vec<String> = scope.iter().map(|b| b.as_str().to_owned()).collect();
            let channel_cond = filter
                .channel
                .map(|c| format!(" AND {}", c.condition("t")))
                .unwrap_or_default();

            // 1. Count completed LIVE lines in the period carrying realized_margin_thb,
            // and sum their realized_margin_thb (§8 ruling: AwaitsLedger applies when no completed
            // LIVE line in the period carries realized_margin_thb).
            let lines_sql = format!(
                "SELECT count(l.id), coalesce(sum(l.realized_margin_thb), 0) \
                   FROM transactions t \
                   JOIN branches b ON b.organization_id = t.organization_id AND b.id = t.branch_id \
                   JOIN transaction_lines l ON l.transaction_id = t.id \
                  WHERE t.status = 'COMPLETED' AND t.mode = 'LIVE' AND t.branch_id = ANY($1) \
                    AND t.value_date >= coalesce($2::date, (now() AT TIME ZONE b.timezone)::date) \
                    AND t.value_date <= coalesce($3::date, (now() AT TIME ZONE b.timezone)::date) \
                    AND ($4::text IS NULL OR l.currency = $4::text){channel_cond} \
                    AND l.realized_margin_thb IS NOT NULL"
            );

            let (count, line_margin): (i64, Decimal) = sqlx::query_as(AssertSqlSafe(lines_sql))
                .bind(&ids)
                .bind(filter.from)
                .bind(filter.to)
                .bind(filter.currency.as_ref().map(|c| c.to_string()))
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;

            if count == 0 {
                return Ok(Err(MetricUnavailable::AwaitsLedger));
            }

            // 2. Sum reversed realized margin from POSTED reversals in the period.
            let rev_sql = format!(
                "SELECT coalesce(sum(r.realized_margin_reversed_thb), 0) \
                   FROM reversals r \
                   JOIN branches b ON b.organization_id = r.organization_id AND b.id = r.branch_id \
                   JOIN transactions t ON t.id = r.original_transaction_id \
                  WHERE r.status = 'POSTED' AND t.mode = 'LIVE' AND r.branch_id = ANY($1) \
                    AND (r.posted_at AT TIME ZONE b.timezone)::date >= coalesce($2::date, (now() AT TIME ZONE b.timezone)::date) \
                    AND (r.posted_at AT TIME ZONE b.timezone)::date <= coalesce($3::date, (now() AT TIME ZONE b.timezone)::date) \
                    AND ($4::text IS NULL OR EXISTS (SELECT 1 FROM transaction_lines l WHERE l.transaction_id = t.id AND l.currency = $4::text)){channel_cond} \
                    AND r.realized_margin_reversed_thb IS NOT NULL"
            );

            let reversed_margin: Decimal = sqlx::query_scalar(AssertSqlSafe(rev_sql))
                .bind(&ids)
                .bind(filter.from)
                .bind(filter.to)
                .bind(filter.currency.as_ref().map(|c| c.to_string()))
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;

            let net_margin = line_margin - reversed_margin;
            let minor = (net_margin * Decimal::from(100))
                .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
                .to_i128()
                .ok_or_else(|| AppError::Internal("margin amount out of range".into()))?;

            let money = Money::try_from_minor_i128(minor.max(0)).map_err(|_| {
                AppError::Internal(format!("invalid money amount from {net_margin}"))
            })?;

            Ok(Ok(money))
        })
    }
}
