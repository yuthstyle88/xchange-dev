//! Today's `CashFacts` (design `executive-finance.md` §3 `available_cash`): on hand is the open shifts'
//! `shift_balances.expected` plus the safe (`ops_state.safe`, minor units per currency); in transit is the
//! DISPATCHED `cash_transfer_lines` sent from the branch. Realized margin waits for the ledger.
//!
//! EX-7 (`LedgerCashFacts`) must replace this reader before LG-6.m #1047, whose done criterion greps for it.

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

use crate::application::executive::metrics::MetricUnavailable;
use crate::application::executive::ports::{BoxFuture, CashFacts, ExecutiveFilter, ShiftStatus};

pub struct ShiftBalanceCashFacts {
    pool: PgPool,
}

impl ShiftBalanceCashFacts {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// On hand per currency of the branch `$1`: open shifts' expected plus the safe.
const ON_HAND: &str = "SELECT code, sum(amount) FROM ( \
       SELECT sb.code::text AS code, sb.expected AS amount \
         FROM shift_balances sb JOIN shifts s ON s.id = sb.shift_id \
        WHERE s.branch_id = $1 AND s.status = 'OPEN' \
       UNION ALL \
       SELECT safe.key, (safe.value #>> '{}')::numeric / 100 \
         FROM ops_state o, jsonb_each(coalesce(o.safe, '{}'::jsonb)) AS safe \
        WHERE o.branch_id = $1 \
     ) h GROUP BY code ORDER BY code";

impl CashFacts for ShiftBalanceCashFacts {
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
                "SELECT coalesce(sum(cl.qty_sent * d.face_value), 0) \
                   FROM cash_transfers ct \
                   JOIN cash_transfer_lines cl ON cl.transfer_id = ct.id \
                   JOIN denominations d ON d.id = cl.denomination_id \
                  WHERE ct.from_branch_id = $1 AND ct.status = 'DISPATCHED' AND cl.currency = 'THB'",
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
            let rows: Vec<(String, Decimal)> = sqlx::query_as(ON_HAND)
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
        _scope: &'a [BranchId],
        _filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<Money, MetricUnavailable>>> {
        Box::pin(async { Ok(Err(MetricUnavailable::AwaitsLedger)) })
    }
}
