//! PgDenominationGroups implementation (§2, §3.2).

use futures::future::BoxFuture;
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::money::CurrencyCode;

use crate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use crate::domain::draft::rate_engine_error::RateEngineError;

#[derive(Debug, Clone)]
pub struct PgDenominationGroups {
    pool: PgPool,
}

impl PgDenominationGroups {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl DenominationGroups for PgDenominationGroups {
    fn groups_of<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        code: CurrencyCode,
    ) -> BoxFuture<'a, Result<Vec<GroupCode>, RateEngineError>> {
        Box::pin(async move {
            let rows: Vec<(String,)> = sqlx::query_as(
                "SELECT DISTINCT group_code FROM denominations \
                 WHERE currency_code = $1 ORDER BY group_code ASC",
            )
            .bind(code.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let mut groups = Vec::with_capacity(rows.len());
            for (g,) in rows {
                let group_code = GroupCode::new(g.trim()).map_err(RateEngineError::Internal)?;
                groups.push(group_code);
            }
            Ok(groups)
        })
    }

    fn source_of<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        code: CurrencyCode,
        group: &'a GroupCode,
        provider: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, RateEngineError>> {
        Box::pin(async move {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT instrument_key FROM denomination_group_sources \
                 WHERE currency_code = $1 AND group_code = $2 AND provider = $3",
            )
            .bind(code.as_str())
            .bind(group.as_str())
            .bind(provider)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            Ok(row.map(|(k,)| k))
        })
    }
}
