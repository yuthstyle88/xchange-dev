//! Reference base lookup for drafts (rates.md §4.1, §12.4, §12.6, SP-2).

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;

pub struct ReferenceBase;

impl ReferenceBase {
    /// Reference base lookup for a currency in a draft.
    ///
    /// The AUTO base of a line is the newest 'BOT' reference line of its currency.
    /// If no 'BOT' line has ever been written for that currency, it is the newest
    /// 'START' line of the branch's organisation. Once BoT has quoted a currency,
    /// its starting price is never used again (rates.md §12.6, SP-2).
    pub async fn newest_in(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        currency: &CurrencyCode,
    ) -> Result<Option<Uuid>, RateEngineError> {
        let bot_line: Option<(Uuid,)> = sqlx::query_as(
            "SELECT l.id FROM rate_reference_lines l \
             JOIN rate_reference_batches b ON b.id = l.batch_id \
             WHERE l.currency_code = $1 AND l.reference_type = 'BOT' \
             ORDER BY b.fetched_at DESC LIMIT 1",
        )
        .bind(currency.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if let Some((id,)) = bot_line {
            return Ok(Some(id));
        }

        let start_line: Option<(Uuid,)> = sqlx::query_as(
            "SELECT l.id FROM rate_reference_lines l \
             JOIN rate_reference_batches b ON b.id = l.batch_id \
             JOIN branches br ON br.id = $2 \
             WHERE l.currency_code = $1 \
               AND l.reference_type = 'START' \
               AND b.provider = 'START:' || br.organization_id::text \
             ORDER BY b.fetched_at DESC LIMIT 1",
        )
        .bind(currency.as_str())
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        Ok(start_line.map(|(id,)| id))
    }
}
