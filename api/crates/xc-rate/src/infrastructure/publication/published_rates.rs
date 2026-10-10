//! PgPublishedRates: PostgreSQL implementation of PublishedRates port.

use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ExchangeArithmetic, Money, Rate, Side, UnitSize};

use crate::domain::policy::stale_policy::{OnStale, StalePolicy};
use crate::domain::publication::freshness::Freshness;
use crate::domain::publication::publication::{Publication, PublicationId};
use crate::domain::publication::published_rates::{
    LineSelector, PublicationRef, PublishedRateLineId, PublishedRates, PublishedSide,
};
use crate::domain::publication::value_tier::{TierBand, TierCode, TierTable};

pub struct PgPublishedRates {
    pool: PgPool,
}

impl PgPublishedRates {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PublishedRates for PgPublishedRates {
    fn current<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<Publication>>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT p.id, p.version, p.published_at, p.published_by, \
                        p.approved_request_id::text, p.draft_id::text \
                 FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 WHERE b.id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let r = match row {
                Some(r) => r,
                None => return Ok(None),
            };

            let id: Uuid = r.get(0);
            let version: i32 = r.get(1);
            let published_at: chrono::DateTime<chrono::Utc> = r.get(2);
            let published_by: String = r.get(3);
            let approved_request_id: String = r.get(4);
            let draft_id: Option<String> = r.get(5);

            Ok(Some(Publication {
                id: PublicationId::new(id),
                version,
                published_at,
                published_by,
                approved_request_id,
                draft_id,
            }))
        })
    }

    fn line_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        code: CurrencyCode,
        side: Side,
    ) -> BoxFuture<'a, AppResult<Option<PublishedSide>>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT l.id, l.buy_rate, l.sell_rate, l.unit_size, l.source_snapshot, p.id, p.version \
                 FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 JOIN published_rate_lines l ON l.publication_id = p.id \
                 WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = 'ALL' AND l.value_tier = 'ALL'",
            )
            .bind(branch.as_str())
            .bind(code.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let r = match row {
                Some(r) => r,
                None => return Ok(None),
            };

            let line_id: Uuid = r.get(0);
            let buy_rate: Option<Decimal> = r.get(1);
            let sell_rate: Option<Decimal> = r.get(2);
            let unit_size_dec: Decimal = r.get(3);
            let snapshot: serde_json::Value = r.get(4);
            let pub_id: Uuid = r.get(5);
            let pub_version: i32 = r.get(6);

            let rate_dec = match side {
                Side::Buy => buy_rate,
                Side::Sell => sell_rate,
            };

            let rate = match rate_dec {
                Some(d) => Rate::parse(&d.to_string())
                    .map_err(|e| pos_error::AppError::Internal(format!("invalid rate: {e}")))?,
                None => return Ok(None),
            };

            let unit_size = UnitSize::parse(&unit_size_dec.to_string())
                .map_err(|e| pos_error::AppError::Internal(format!("invalid unit size: {e}")))?;

            let freshness = Freshness::of_side(&snapshot, side, chrono::Utc::now());

            Ok(Some(PublishedSide {
                line_id: PublishedRateLineId::new(line_id),
                rate,
                unit_size,
                publication: PublicationRef {
                    id: PublicationId::new(pub_id),
                    version: pub_version,
                },
                stale: freshness.stale,
                source_date: freshness.source_date,
                value_tier: TierCode::new("ALL").expect("valid tier code"),
            }))
        })
    }

    fn line_for<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        sel: LineSelector,
    ) -> BoxFuture<'a, AppResult<Option<PublishedSide>>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT l.id, l.buy_rate, l.sell_rate, l.unit_size, l.source_snapshot, p.id, p.version, \
                        l.value_tier, l.tier_lower_exclusive_thb, l.tier_upper_inclusive_thb, \
                        l.base_buy_rate, l.base_sell_rate \
                 FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 JOIN published_rate_lines l ON l.publication_id = p.id \
                 WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = $3",
            )
            .bind(branch.as_str())
            .bind(sel.code.as_str())
            .bind(sel.group.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            if rows.is_empty() {
                return Ok(None);
            }

            // Case 1: One line with value_tier = 'ALL' (untiered): that line. quantity is not needed.
            if rows.len() == 1 {
                let tier_str: String = rows[0].get(7);
                if tier_str == "ALL" {
                    let r = &rows[0];
                    let line_id: Uuid = r.get(0);
                    let buy_rate: Option<Decimal> = r.get(1);
                    let sell_rate: Option<Decimal> = r.get(2);
                    let unit_size_dec: Decimal = r.get(3);
                    let snapshot: serde_json::Value = r.get(4);
                    let pub_id: Uuid = r.get(5);
                    let pub_version: i32 = r.get(6);

                    let rate_dec = match sel.side {
                        Side::Buy => buy_rate,
                        Side::Sell => sell_rate,
                    };

                    let rate = match rate_dec {
                        Some(d) => Rate::parse(&d.to_string()).map_err(|e| {
                            pos_error::AppError::Internal(format!("invalid rate: {e}"))
                        })?,
                        None => return Ok(None),
                    };

                    let unit_size = UnitSize::parse(&unit_size_dec.to_string()).map_err(|e| {
                        pos_error::AppError::Internal(format!("invalid unit size: {e}"))
                    })?;

                    let freshness = Freshness::of_side(&snapshot, sel.side, chrono::Utc::now());

                    return Ok(Some(PublishedSide {
                        line_id: PublishedRateLineId::new(line_id),
                        rate,
                        unit_size,
                        publication: PublicationRef {
                            id: PublicationId::new(pub_id),
                            version: pub_version,
                        },
                        stale: freshness.stale,
                        source_date: freshness.source_date,
                        value_tier: TierCode::new("ALL").expect("valid tier code"),
                    }));
                }
            }

            // Case 2: Otherwise, tiered:
            // 1. No quantity -> None.
            let quantity = match sel.quantity {
                Some(q) => q,
                None => return Ok(None),
            };

            // 2. The base rate of the side is base_buy_rate for BUY and base_sell_rate for SELL.
            // It must be present and the same on every tier line of (code, group); otherwise -> None.
            let mut common_base: Option<Decimal> = None;
            for r in &rows {
                let base_dec: Option<Decimal> = match sel.side {
                    Side::Buy => r.get(10),
                    Side::Sell => r.get(11),
                };
                let base_dec = match base_dec {
                    Some(d) => d,
                    None => return Ok(None),
                };
                match common_base {
                    Some(prev) => {
                        if prev != base_dec {
                            return Ok(None);
                        }
                    }
                    None => common_base = Some(base_dec),
                }
            }
            let base_dec = match common_base {
                Some(d) => d,
                None => return Ok(None),
            };

            // 3. base_value = ExchangeArithmetic::thb(quantity, base_rate, unit_size).
            let unit_size_dec: Decimal = rows[0].get(3);
            let unit_size = UnitSize::parse(&unit_size_dec.to_string())
                .map_err(|e| pos_error::AppError::Internal(format!("invalid unit size: {e}")))?;
            let base_rate = Rate::parse(&base_dec.to_string())
                .map_err(|e| pos_error::AppError::Internal(format!("invalid base rate: {e}")))?;
            let base_value = ExchangeArithmetic::thb(quantity, base_rate, unit_size);

            // 4. TierTable is built from the lines' bands. TierTable::pick(base_value) -> the line of that tier.
            let mut bands = Vec::with_capacity(rows.len());
            for r in &rows {
                let tier_str: String = r.get(7);
                let tier_code = TierCode::new(tier_str).map_err(|e| {
                    pos_error::AppError::Internal(format!("invalid tier code: {e}"))
                })?;
                let lower_dec: Option<Decimal> = r.get(8);
                let upper_dec: Option<Decimal> = r.get(9);
                let lower_exclusive_thb = match lower_dec {
                    Some(d) => Some(Money::parse(&d.to_string()).map_err(|e| {
                        pos_error::AppError::Internal(format!("invalid lower bound: {e}"))
                    })?),
                    None => None,
                };
                let upper_inclusive_thb = match upper_dec {
                    Some(d) => Some(Money::parse(&d.to_string()).map_err(|e| {
                        pos_error::AppError::Internal(format!("invalid upper bound: {e}"))
                    })?),
                    None => None,
                };
                bands.push(TierBand {
                    code: tier_code,
                    lower_exclusive_thb,
                    upper_inclusive_thb,
                });
            }

            let table = TierTable::new(bands);
            let picked_tier = match table.pick(base_value) {
                Some(t) => t,
                None => return Ok(None),
            };

            let picked_row = match rows.iter().find(|r| {
                let t: String = r.get(7);
                t == picked_tier.as_str()
            }) {
                Some(r) => r,
                None => return Ok(None),
            };

            // 5. The answer is that line's side. If that side is NULL -> None. Never fall back to another tier.
            let line_id: Uuid = picked_row.get(0);
            let buy_rate: Option<Decimal> = picked_row.get(1);
            let sell_rate: Option<Decimal> = picked_row.get(2);
            let picked_unit_size_dec: Decimal = picked_row.get(3);
            let snapshot: serde_json::Value = picked_row.get(4);
            let pub_id: Uuid = picked_row.get(5);
            let pub_version: i32 = picked_row.get(6);

            let rate_dec = match sel.side {
                Side::Buy => buy_rate,
                Side::Sell => sell_rate,
            };

            let rate = match rate_dec {
                Some(d) => Rate::parse(&d.to_string())
                    .map_err(|e| pos_error::AppError::Internal(format!("invalid rate: {e}")))?,
                None => return Ok(None),
            };

            let unit_size = UnitSize::parse(&picked_unit_size_dec.to_string())
                .map_err(|e| pos_error::AppError::Internal(format!("invalid unit size: {e}")))?;

            let freshness = Freshness::of_side(&snapshot, sel.side, chrono::Utc::now());

            Ok(Some(PublishedSide {
                line_id: PublishedRateLineId::new(line_id),
                rate,
                unit_size,
                publication: PublicationRef {
                    id: PublicationId::new(pub_id),
                    version: pub_version,
                },
                stale: freshness.stale,
                source_date: freshness.source_date,
                value_tier: picked_tier,
            }))
        })
    }

    fn line_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        line_id: PublishedRateLineId,
        side: Side,
    ) -> BoxFuture<'a, AppResult<Option<PublishedSide>>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT l.id, l.buy_rate, l.sell_rate, l.unit_size, l.source_snapshot, p.id, p.version, l.value_tier \
                 FROM branches b \
                 JOIN rate_publications p ON p.id = b.current_publication_id \
                 JOIN published_rate_lines l ON l.publication_id = p.id \
                 WHERE b.id = $1 AND l.id = $2",
            )
            .bind(branch.as_str())
            .bind(line_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let r = match row {
                Some(r) => r,
                None => return Ok(None),
            };

            let row_line_id: Uuid = r.get(0);
            let buy_rate: Option<Decimal> = r.get(1);
            let sell_rate: Option<Decimal> = r.get(2);
            let unit_size_dec: Decimal = r.get(3);
            let snapshot: serde_json::Value = r.get(4);
            let pub_id: Uuid = r.get(5);
            let pub_version: i32 = r.get(6);
            let tier_str: String = r.get(7);

            let rate_dec = match side {
                Side::Buy => buy_rate,
                Side::Sell => sell_rate,
            };

            let rate = match rate_dec {
                Some(d) => Rate::parse(&d.to_string())
                    .map_err(|e| pos_error::AppError::Internal(format!("invalid rate: {e}")))?,
                None => return Ok(None),
            };

            let unit_size = UnitSize::parse(&unit_size_dec.to_string())
                .map_err(|e| pos_error::AppError::Internal(format!("invalid unit size: {e}")))?;

            let freshness = Freshness::of_side(&snapshot, side, chrono::Utc::now());
            let value_tier = TierCode::new(tier_str)
                .map_err(|e| pos_error::AppError::Internal(format!("invalid tier code: {e}")))?;

            Ok(Some(PublishedSide {
                line_id: PublishedRateLineId::new(row_line_id),
                rate,
                unit_size,
                publication: PublicationRef {
                    id: PublicationId::new(pub_id),
                    version: pub_version,
                },
                stale: freshness.stale,
                source_date: freshness.source_date,
                value_tier,
            }))
        })
    }

    fn stale_policy_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        line: &'a PublishedSide,
    ) -> BoxFuture<'a, AppResult<StalePolicy>> {
        Box::pin(async move {
            let snapshot: Option<serde_json::Value> = sqlx::query_scalar(
                "SELECT source_snapshot FROM published_rate_lines WHERE id = $1",
            )
            .bind(line.line_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let snapshot = snapshot.ok_or_else(|| {
                pos_error::AppError::Internal(format!(
                    "published rate line {} not found",
                    line.line_id
                ))
            })?;

            let policy_id_str = snapshot
                .get("policy_version_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    pos_error::AppError::Internal(format!(
                        "line {} has no policy_version_id in source_snapshot",
                        line.line_id
                    ))
                })?;

            let policy_id = Uuid::parse_str(policy_id_str).map_err(|e| {
                pos_error::AppError::Internal(format!("invalid policy_version_id: {e}"))
            })?;

            let row: Option<(Option<String>, Option<Decimal>)> = sqlx::query_as(
                "SELECT on_stale, stale_limit_thb FROM rate_policy_versions WHERE id = $1",
            )
            .bind(policy_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let (on_stale_str, stale_limit_dec) = row.ok_or_else(|| {
                pos_error::AppError::Internal(format!("policy version {policy_id} not found"))
            })?;

            let on_stale = match on_stale_str {
                Some(s) => Some(OnStale::parse_db(&s).ok_or_else(|| {
                    pos_error::AppError::Internal(format!("invalid on_stale: {s}"))
                })?),
                None => None,
            };

            let stale_limit_thb = match stale_limit_dec {
                Some(d) => Some(Money::parse(&d.to_string()).map_err(|e| {
                    pos_error::AppError::Internal(format!("invalid stale_limit_thb: {e}"))
                })?),
                None => None,
            };

            Ok(StalePolicy::new(on_stale, stale_limit_thb))
        })
    }
}
