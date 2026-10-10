//! `ExchangeFeedWriter` (design §6.2, §6.3): writes `broker_exchange_feed` in the caller's
//! transaction. FD-2's save step calls `record_completed`; #552's reversal calls `record_reversed`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::customer_keys::CustomerKeys;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedExchange {
    pub transaction_id: Uuid,
    /// `organizations.id` (a uuid) as text.
    pub organization_id: String,
    pub branch_id: String,
    pub keys: CustomerKeys,
    /// From `FeedValue`; > 0.
    pub value_satang: i64,
    pub completed_at: DateTime<Utc>,
}

pub struct ExchangeFeedWriter;

impl ExchangeFeedWriter {
    /// `INSERT … ON CONFLICT (transaction_id) DO NOTHING`, `updated_at = clock_timestamp()`. A
    /// repeat changes nothing, `updated_at` included.
    pub async fn record_completed(
        tx: &mut Transaction<'_, Postgres>,
        exchange: &CompletedExchange,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO broker_exchange_feed ( \
                 transaction_id, organization_id, branch_id, customer_sub, phone, \
                 status, value_satang, reversed_satang, completed_at, reversed_at, updated_at \
             ) VALUES ($1::uuid, $2::uuid, $3, $4, $5, 'COMPLETED', $6, 0, $7, NULL, clock_timestamp()) \
             ON CONFLICT (transaction_id) DO NOTHING",
        )
        .bind(exchange.transaction_id.to_string())
        .bind(&exchange.organization_id)
        .bind(&exchange.branch_id)
        .bind(&exchange.keys.customer_sub)
        .bind(&exchange.keys.phone)
        .bind(exchange.value_satang)
        .bind(exchange.completed_at)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(())
    }

    /// Sets the **cumulative** reversed amount (never a delta), `status = REVERSED`,
    /// `reversed_at = at` and a new `updated_at`.
    /// - `reversed_total` not in `1..=value_satang` ⇒ `Err(AppError::Validation(..))`, nothing written;
    /// - no row for `transaction_id` ⇒ `Err(AppError::NotFound { entity, id })`.
    pub async fn record_reversed(
        tx: &mut Transaction<'_, Postgres>,
        transaction_id: Uuid,
        reversed_total: i64,
        at: DateTime<Utc>,
    ) -> AppResult<()> {
        let row = sqlx::query_scalar::<_, i64>(
            "SELECT value_satang FROM broker_exchange_feed WHERE transaction_id = $1::uuid FOR UPDATE",
        )
        .bind(transaction_id.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        let Some(value_satang) = row else {
            return Err(pos_error::AppError::NotFound {
                entity: "broker_exchange_feed",
                id: transaction_id.to_string(),
            });
        };

        if !(1..=value_satang).contains(&reversed_total) {
            return Err(pos_error::AppError::Validation(format!(
                "reversed amount {reversed_total} must be between 1 and {value_satang}"
            )));
        }

        sqlx::query(
            "UPDATE broker_exchange_feed \
             SET status = 'REVERSED', \
                 reversed_satang = $1, \
                 reversed_at = $2, \
                 updated_at = clock_timestamp() \
             WHERE transaction_id = $3::uuid",
        )
        .bind(reversed_total)
        .bind(at)
        .bind(transaction_id.to_string())
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(())
    }
}
