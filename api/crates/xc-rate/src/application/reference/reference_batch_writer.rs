//! ReferenceBatchWriter application service (§4.1, §4.2, §4.7).

use std::sync::Arc;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::domain::reference::reference_provider::ReferenceProvider;
use crate::infrastructure::reference::bot_provider::BotProvider;
use crate::infrastructure::reference::pg_reference_batch_store::{
    PgReferenceBatchStore, ReferenceBatchRecord, ReferenceLineRecord,
};

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub struct ReferenceBatchWriter {
    source: Arc<dyn RateSource>,
    store: Arc<PgReferenceBatchStore>,
    provider: Arc<dyn ReferenceProvider>,
}

impl ReferenceBatchWriter {
    pub fn new(source: Arc<dyn RateSource>, store: Arc<PgReferenceBatchStore>) -> Self {
        Self {
            source,
            store,
            provider: Arc::new(BotProvider),
        }
    }

    pub fn with_source(mut self, source: Arc<dyn RateSource>) -> Self {
        self.source = source;
        self
    }

    pub fn with_provider(mut self, provider: Arc<dyn ReferenceProvider>) -> Self {
        self.provider = provider;
        self
    }

    pub async fn fetch_and_write(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Uuid, RateEngineError> {
        let rates = self
            .source
            .fetch()
            .await
            .map_err(|e| RateEngineError::RateUnavailable(e.to_string()))?;

        // Canonical checksum over rows, sorted by provider's row_key
        let mut sorted_rows = rates.rows.clone();
        sorted_rows.sort_by_cached_key(|r| self.provider.row_key(r));
        let rows_bytes = serde_json::to_vec(&sorted_rows)
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;
        let raw_checksum = sha256_hex(&rows_bytes);

        // Deduplicate: same checksum for this provider returns existing id without writing
        if let Some(existing_id) = self
            .store
            .find_existing_by_checksum(tx, self.provider.provider(), &raw_checksum)
            .await?
        {
            return Ok(existing_id);
        }

        let parsed_batch = self.provider.parse(&rates)?;

        let fetched_at = chrono::DateTime::parse_from_rfc3339(&rates.fetched_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        let status = if parsed_batch.complete {
            "COMPLETE"
        } else {
            "PARTIAL"
        };

        let batch_id = Uuid::new_v4();
        let line_records: Vec<ReferenceLineRecord> = parsed_batch
            .lines
            .into_iter()
            .map(|l| ReferenceLineRecord {
                id: Uuid::new_v4(),
                batch_id,
                currency_code: l.currency,
                reference_type: self.provider.reference_type().to_string(),
                unit_size: l.unit_size,
                buy: l.buy,
                sell: l.sell,
                raw: l.raw,
            })
            .collect();

        let batch_record = ReferenceBatchRecord {
            id: batch_id,
            provider: self.provider.provider().to_string(),
            source_date: parsed_batch.source_date,
            fetched_at,
            raw_checksum,
            status: status.to_string(),
        };

        self.store
            .insert_batch(tx, &batch_record, &line_records)
            .await?;
        Ok(batch_id)
    }
}
