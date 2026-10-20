//! Reference base lookup for drafts (rates.md §4.1, §12.4, §12.6, SP-2; pricing-tiers.md §3.1, PT-15).

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::anchor_source::AnchorSource;

pub struct ReferenceBase;

impl ReferenceBase {
    /// Reference base lookup for a currency in a draft.
    ///
    /// Resolves the effective AUTO anchor for `(branch, currency)` from the branch's current
    /// policy version (`COALESCE(s.anchor_source, v.anchor_source)`, or `BOT` if no current
    /// version). For `BOT`, returns the newest `'BOT'` line, falling back to the org's newest
    /// `'START'` line if no `'BOT'` line exists. For `MARKET_TT` / `MARKET_SR`, returns the
    /// newest line of that `reference_type` with no fallback to `BOT` or `START`.
    pub async fn newest_in(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        currency: &CurrencyCode,
    ) -> Result<Option<Uuid>, RateEngineError> {
        let anchor_row: Option<(String,)> = sqlx::query_as(
            "SELECT COALESCE(s.anchor_source, v.anchor_source) \
             FROM rate_policy_versions v \
             LEFT JOIN rate_policy_currency_settings s \
               ON s.policy_version_id = v.id AND s.currency_code = $2 \
             WHERE v.branch_id = $1 \
               AND v.valid_from <= now() \
               AND (v.valid_to IS NULL OR v.valid_to > now()) \
             ORDER BY v.version DESC LIMIT 1",
        )
        .bind(branch.as_str())
        .bind(currency.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let anchor = match anchor_row {
            Some((raw,)) => AnchorSource::parse(&raw).ok_or_else(|| {
                RateEngineError::Internal(format!("unknown anchor_source: {raw}"))
            })?,
            None => AnchorSource::Bot,
        };

        let anchor_line: Option<(Uuid,)> = sqlx::query_as(
            "SELECT l.id FROM rate_reference_lines l \
             JOIN rate_reference_batches b ON b.id = l.batch_id \
             WHERE l.currency_code = $1 AND l.reference_type = $2 \
             ORDER BY b.fetched_at DESC LIMIT 1",
        )
        .bind(currency.as_str())
        .bind(anchor.reference_type())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if let Some((id,)) = anchor_line {
            return Ok(Some(id));
        }

        if anchor != AnchorSource::Bot {
            return Ok(None);
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
