//! Reference base lookup for drafts (rates.md §4.1, §12.4).

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;

pub struct ReferenceBase;

impl ReferenceBase {
    /// Pure move of the newest 'BOT' reference lookup for a currency in a draft.
    /// `_branch` is kept in the signature for SP-2.
    pub async fn newest_in(
        tx: &mut Transaction<'_, Postgres>,
        _branch: &BranchId,
        currency: &CurrencyCode,
    ) -> Result<Option<Uuid>, RateEngineError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT l.id FROM rate_reference_lines l \
             JOIN rate_reference_batches b ON b.id = l.batch_id \
             WHERE l.currency_code = $1 AND l.reference_type = 'BOT' \
             ORDER BY b.fetched_at DESC LIMIT 1",
        )
        .bind(currency.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        Ok(row.map(|(id,)| id))
    }
}
