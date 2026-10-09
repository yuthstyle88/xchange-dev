//! ReferenceBatchWriter application service (§4.1, §4.2, §4.7).

use std::str::FromStr;
use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::infrastructure::reference::pg_reference_batch_store::{
    PgReferenceBatchStore, ReferenceBatchRecord, ReferenceLineRecord,
};

const BOT_CURRENCIES: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn parse_decimal(val: &serde_json::Value) -> Option<Decimal> {
    match val {
        serde_json::Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
        serde_json::Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

pub struct ReferenceBatchWriter {
    source: Arc<dyn RateSource>,
    store: Arc<PgReferenceBatchStore>,
}

impl ReferenceBatchWriter {
    pub fn new(source: Arc<dyn RateSource>, store: Arc<PgReferenceBatchStore>) -> Self {
        Self { source, store }
    }

    pub fn with_source(mut self, source: Arc<dyn RateSource>) -> Self {
        self.source = source;
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

        // Canonical checksum over rows
        let mut sorted_rows = rates.rows.clone();
        sorted_rows.sort_by_key(|r| {
            r.get("code")
                .or_else(|| r.get("currency_code"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        });
        let rows_bytes = serde_json::to_vec(&sorted_rows)
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;
        let raw_checksum = sha256_hex(&rows_bytes);

        // Deduplicate: same checksum returns existing id without writing
        if let Some(existing_id) = self
            .store
            .find_existing_by_checksum(tx, "BOT", &raw_checksum)
            .await?
        {
            return Ok(existing_id);
        }

        let mut source_date = None;
        for r in &rates.rows {
            if let Some(d_str) = r
                .get("date")
                .or_else(|| r.get("period"))
                .and_then(|v| v.as_str())
            {
                if let Ok(nd) = chrono::NaiveDate::parse_from_str(d_str, "%Y-%m-%d") {
                    source_date = Some(nd);
                    break;
                }
            }
        }

        let fetched_at = chrono::DateTime::parse_from_rfc3339(&rates.fetched_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        let source_date = source_date.unwrap_or_else(|| fetched_at.date_naive());

        let mut present_currencies = std::collections::HashSet::new();
        for r in &rates.rows {
            if let Some(code) = r
                .get("code")
                .or_else(|| r.get("currency_code"))
                .and_then(|v| v.as_str())
            {
                present_currencies.insert(code.trim().to_uppercase());
            }
        }

        let is_complete = BOT_CURRENCIES
            .iter()
            .all(|c| present_currencies.contains(*c));
        let status = if is_complete { "COMPLETE" } else { "PARTIAL" };

        let batch_id = Uuid::new_v4();
        let mut line_records = Vec::new();
        let mut seen_line_currencies = std::collections::HashSet::new();

        for r in &rates.rows {
            let Some(code_raw) = r
                .get("code")
                .or_else(|| r.get("currency_code"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let code = code_raw.trim().to_uppercase();
            if !seen_line_currencies.insert(code.clone()) {
                continue;
            }

            let unit_size = r
                .get("unit")
                .or_else(|| r.get("unit_size"))
                .and_then(|v| {
                    if let Some(u) = v.as_u64() {
                        Some(Decimal::from(u))
                    } else if let Some(s) = v.as_str() {
                        Decimal::from_str(s).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(Decimal::ONE);

            let buy = r
                .get("buyingTransfer")
                .or_else(|| r.get("buying_transfer"))
                .or_else(|| r.get("buy"))
                .and_then(parse_decimal);

            let sell = r
                .get("selling")
                .or_else(|| r.get("sell"))
                .and_then(parse_decimal);

            line_records.push(ReferenceLineRecord {
                id: Uuid::new_v4(),
                batch_id,
                currency_code: code,
                reference_type: "BOT".to_string(),
                unit_size,
                buy,
                sell,
                raw: r.clone(),
            });
        }

        let batch_record = ReferenceBatchRecord {
            id: batch_id,
            provider: "BOT".to_string(),
            source_date,
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
