//! PgReferenceBatchStore: PostgreSQL implementation for reference batches and lines.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::draft::rate_engine_error::RateEngineError;

#[derive(Debug, Clone)]
pub struct ReferenceBatchRecord {
    pub id: Uuid,
    pub provider: String,
    pub source_date: NaiveDate,
    pub fetched_at: DateTime<Utc>,
    pub raw_checksum: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct ReferenceLineRecord {
    pub id: Uuid,
    pub batch_id: Uuid,
    pub currency_code: String,
    pub reference_type: String,
    pub unit_size: Decimal,
    pub buy: Option<Decimal>,
    pub sell: Option<Decimal>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Default, Clone)]
pub struct PgReferenceBatchStore;

impl PgReferenceBatchStore {
    pub fn new() -> Self {
        Self
    }

    pub async fn find_existing_by_checksum(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        provider: &str,
        checksum: &str,
    ) -> Result<Option<Uuid>, RateEngineError> {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM rate_reference_batches \
             WHERE provider = $1 AND raw_checksum = $2 \
             ORDER BY fetched_at DESC, id DESC LIMIT 1",
        )
        .bind(provider)
        .bind(checksum)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        Ok(row.map(|r| r.0))
    }

    /// Sets the batch's `fetched_at` to `fetched_at`: a repeated answer confirms it (pricing-tiers.md §4, XC-PRICING
    /// PX-12).
    pub async fn confirm(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        batch_id: Uuid,
        fetched_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), RateEngineError> {
        sqlx::query("UPDATE rate_reference_batches SET fetched_at = $2 WHERE id = $1")
            .bind(batch_id)
            .bind(fetched_at)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        Ok(())
    }

    pub async fn insert_batch(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        batch: &ReferenceBatchRecord,
        lines: &[ReferenceLineRecord],
    ) -> Result<Uuid, RateEngineError> {
        sqlx::query(
            "INSERT INTO rate_reference_batches \
             (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(batch.id)
        .bind(&batch.provider)
        .bind(batch.source_date)
        .bind(batch.fetched_at)
        .bind(&batch.raw_checksum)
        .bind(&batch.status)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        for line in lines {
            sqlx::query(
                "INSERT INTO rate_reference_lines \
                 (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(line.id)
            .bind(line.batch_id)
            .bind(&line.currency_code)
            .bind(&line.reference_type)
            .bind(line.unit_size)
            .bind(line.buy)
            .bind(line.sell)
            .bind(&line.raw)
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        }

        Ok(batch.id)
    }
}
