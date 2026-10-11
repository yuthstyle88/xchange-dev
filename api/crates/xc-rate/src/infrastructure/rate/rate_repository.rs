//! Rate repository postgres implementation: the currency register and a branch's current
//! publication per 1 unit (RE-7 #604: no `rates` table any more).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Rate;

use crate::domain::rate::repositories::rate_repository::{CurrentRateRow, RateRepository};
use crate::domain::validation::decimal_to_rate;

type PublishedLineDbRow = (
    String,
    Option<Decimal>,
    Option<Decimal>,
    Decimal,
    serde_json::Value,
    DateTime<Utc>,
);

#[derive(Clone)]
pub struct PgRateRepository {
    pool: PgPool,
}

impl PgRateRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A published figure per 1 unit (`rate / unit_size`, at most 8 places); `None` for an
/// unpublished side or a figure that is not a rate.
fn per_one(rate: Option<Decimal>, unit: Decimal) -> AppResult<Option<Rate>> {
    match rate {
        Some(r) if !unit.is_zero() && r > Decimal::ZERO => {
            decimal_to_rate(&(r / unit).round_dp(8)).map(Some)
        }
        _ => Ok(None),
    }
}

/// `source_snapshot.partner`'s `name` and `cost` (#604 ruling Q2): `("", None)` without one.
fn partner_of(snapshot: &serde_json::Value) -> AppResult<(String, Option<Rate>)> {
    let Some(partner) = snapshot.get("partner").filter(|p| p.is_object()) else {
        return Ok((String::new(), None));
    };
    let name = partner
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    let cost: Option<Decimal> = match partner.get("cost") {
        Some(serde_json::Value::String(text)) => text.parse().ok(),
        Some(serde_json::Value::Number(n)) => n.to_string().parse().ok(),
        _ => None,
    };
    let cost = match cost {
        Some(c) if c > Decimal::ZERO => Some(decimal_to_rate(&c)?),
        _ => None,
    };
    Ok((name, cost))
}

impl RateRepository for PgRateRepository {
    async fn list_currencies(&self) -> AppResult<Vec<(String, String, Option<String>)>> {
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT code, name_th, symbol FROM currencies WHERE code != 'THB'",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)
    }

    async fn current_rates(&self, branch: &BranchId) -> AppResult<Vec<CurrentRateRow>> {
        let rows = sqlx::query_as::<_, PublishedLineDbRow>(
            "SELECT DISTINCT ON (l.currency_code) \
                    l.currency_code, l.buy_rate, l.sell_rate, l.unit_size, l.source_snapshot, \
                    p.published_at \
             FROM branches b \
             JOIN rate_publications p ON p.id = b.current_publication_id \
             JOIN published_rate_lines l ON l.publication_id = p.id \
             WHERE b.id = $1 AND l.denomination_group = 'ALL' \
             ORDER BY l.currency_code, l.tier_lower_exclusive_thb ASC NULLS FIRST",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut results = Vec::with_capacity(rows.len());
        for (code, buy, sell, unit, snapshot, published_at) in rows {
            let (partner_name, partner_cost) = partner_of(&snapshot)?;
            results.push(CurrentRateRow {
                code: code.trim().to_owned(),
                buy: per_one(buy, unit)?,
                sell: per_one(sell, unit)?,
                updated_at: published_at,
                partner_name,
                partner_cost,
            });
        }
        Ok(results)
    }
}
