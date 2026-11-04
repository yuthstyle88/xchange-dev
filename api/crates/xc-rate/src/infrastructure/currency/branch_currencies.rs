//! `PgBranchCurrencies` (`docs/design/board-currencies.md` §8).

use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::currency::branch_currencies::{BranchCurrencies, BranchCurrency, Display};
use crate::domain::currency::denomination_groups::GroupCode;
use crate::domain::draft::rate_engine_error::RateEngineError;

#[derive(Debug, Clone, Copy, Default)]
pub struct PgBranchCurrencies;

fn display_of(text: &str) -> Result<Display, RateEngineError> {
    Display::parse(text)
        .ok_or_else(|| RateEngineError::Internal(format!("unknown display: {text}")))
}

impl BranchCurrencies for PgBranchCurrencies {
    fn rows_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, Result<Vec<BranchCurrency>, RateEngineError>> {
        Box::pin(async move {
            let rows: Vec<(String, String, String, i32)> = sqlx::query_as(
                "SELECT currency_code::text, group_code, display, sort_order \
                 FROM branch_currencies WHERE branch_id = $1 \
                 ORDER BY sort_order, currency_code, group_code",
            )
            .bind(branch.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            rows.into_iter()
                .map(|(code, group, display, sort_order)| {
                    Ok(BranchCurrency {
                        code: CurrencyCode::parse(code.trim())
                            .map_err(|e| RateEngineError::Internal(format!("{code}: {e}")))?,
                        group: GroupCode::new(group).map_err(RateEngineError::Internal)?,
                        display: display_of(&display)?,
                        sort_order,
                    })
                })
                .collect()
        })
    }

    fn display_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        code: CurrencyCode,
        group: &'a GroupCode,
    ) -> BoxFuture<'a, Result<Option<Display>, RateEngineError>> {
        Box::pin(async move {
            let row: Option<String> = sqlx::query_scalar(
                "SELECT display FROM branch_currencies \
                 WHERE branch_id = $1 AND currency_code = $2 AND group_code = $3",
            )
            .bind(branch.as_str())
            .bind(code.as_str())
            .bind(group.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            row.as_deref().map(display_of).transpose()
        })
    }
}
