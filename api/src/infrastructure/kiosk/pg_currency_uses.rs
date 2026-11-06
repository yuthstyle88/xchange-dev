//! The counts behind the kiosk's popular currencies (`application::kiosk_popular`): one query over
//! `transactions` (indexed by `transactions_branch_status_idx`: `branch_id`, `status`) and its lines
//! (`transaction_lines (transaction_id, line_no)`). No table of its own.

use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::application::kiosk_popular::{BoxFuture, CurrencyUse, CurrencyUseSource};

pub struct PgCurrencyUses {
    pool: PgPool,
}

impl PgCurrencyUses {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CurrencyUseSource for PgCurrencyUses {
    fn uses<'a>(
        &'a self,
        branch: &'a BranchId,
        since: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Vec<CurrencyUse>>> {
        Box::pin(async move {
            let rows: Vec<(String, i64, String)> = sqlx::query_as(
                "SELECT tl.currency::text, count(DISTINCT t.id), COALESCE(sum(tl.thb_amount), 0)::text \
                 FROM transactions t \
                 JOIN transaction_lines tl ON tl.transaction_id = t.id \
                 WHERE t.branch_id = $1 AND t.status = 'COMPLETED' AND t.mode = 'LIVE' \
                   AND t.value_date >= $2 \
                 GROUP BY tl.currency",
            )
            .bind(branch.as_str())
            .bind(since)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Internal(format!("kiosk popular currencies: {e}")))?;
            rows.into_iter()
                .map(|(code, completed, thb)| {
                    let thb = thb.parse::<Decimal>().map_err(|e| {
                        AppError::Internal(format!("kiosk popular currencies: THB {thb:?}: {e}"))
                    })?;
                    Ok(CurrencyUse {
                        code: code.trim().to_owned(),
                        completed,
                        thb,
                    })
                })
                .collect()
        })
    }
}
