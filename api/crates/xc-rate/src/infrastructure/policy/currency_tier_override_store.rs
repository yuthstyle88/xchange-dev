//! Store of per-currency overrides on a policy version (pricing-tiers.md §3, §10.1): `rate_policy_currency_tiers`
//! and `rate_policy_currency_settings`. Rows are immutable once inserted; a new version gets them by `carry_forward`.
//!
//! SKELETON (PT-0): the bodies are PT-3's. The signatures stay.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, RatePercent};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::anchor_source::AnchorSource;
use crate::domain::policy::currency_tier_override::{CurrencyTierOverride, TierMargin};

pub struct CurrencyTierOverrideStore;

impl CurrencyTierOverrideStore {
    /// Every currency override of `policy_version_id`, ordered by currency code. A currency appears when it has a
    /// settings row or tier rows; its `tiers` are `Some` only with exactly four tier rows.
    pub async fn for_version(
        tx: &mut Transaction<'_, Postgres>,
        policy_version_id: &Uuid,
    ) -> Result<Vec<CurrencyTierOverride>, RateEngineError> {
        let tier_rows = sqlx::query(
            "SELECT currency_code::text AS currency_code, tier_no, buy_margin_pct, sell_margin_pct \
             FROM rate_policy_currency_tiers \
             WHERE policy_version_id = $1 \
             ORDER BY currency_code ASC, tier_no ASC",
        )
        .bind(policy_version_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let setting_rows = sqlx::query(
            "SELECT currency_code::text AS currency_code, min_spread_pct, anchor_source \
             FROM rate_policy_currency_settings \
             WHERE policy_version_id = $1 \
             ORDER BY currency_code ASC",
        )
        .bind(policy_version_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        struct Entry {
            currency: CurrencyCode,
            tiers: Vec<TierMargin>,
            min_spread_pct: Option<RatePercent>,
            anchor_source: Option<AnchorSource>,
        }

        let mut by_code: BTreeMap<String, Entry> = BTreeMap::new();

        for row in tier_rows {
            let code_str: String = row.get("currency_code");
            let currency = CurrencyCode::parse(&code_str).map_err(|e| {
                RateEngineError::Internal(format!("invalid currency_code {code_str}: {e}"))
            })?;
            let tier_no: i16 = row.get("tier_no");
            let buy_dec: Decimal = row.get("buy_margin_pct");
            let sell_dec: Decimal = row.get("sell_margin_pct");
            let buy_margin_pct = RatePercent::parse(&buy_dec.to_string()).map_err(|e| {
                RateEngineError::Internal(format!("invalid buy_margin_pct {buy_dec}: {e}"))
            })?;
            let sell_margin_pct = RatePercent::parse(&sell_dec.to_string()).map_err(|e| {
                RateEngineError::Internal(format!("invalid sell_margin_pct {sell_dec}: {e}"))
            })?;

            let entry = by_code.entry(code_str).or_insert_with(|| Entry {
                currency,
                tiers: Vec::with_capacity(4),
                min_spread_pct: None,
                anchor_source: None,
            });
            entry.tiers.push(TierMargin {
                tier_no: tier_no as u8,
                buy_margin_pct,
                sell_margin_pct,
            });
        }

        for row in setting_rows {
            let code_str: String = row.get("currency_code");
            let currency = CurrencyCode::parse(&code_str).map_err(|e| {
                RateEngineError::Internal(format!("invalid currency_code {code_str}: {e}"))
            })?;
            let min_spread_dec: Option<Decimal> = row.get("min_spread_pct");
            let min_spread_pct = match min_spread_dec {
                Some(d) => Some(RatePercent::parse(&d.to_string()).map_err(|e| {
                    RateEngineError::Internal(format!("invalid min_spread_pct {d}: {e}"))
                })?),
                None => None,
            };
            let anchor_str: Option<String> = row.get("anchor_source");
            let anchor_source = match anchor_str {
                Some(s) => Some(AnchorSource::parse(&s).ok_or_else(|| {
                    RateEngineError::Internal(format!("unknown anchor_source {s}"))
                })?),
                None => None,
            };

            let entry = by_code.entry(code_str).or_insert_with(|| Entry {
                currency,
                tiers: Vec::new(),
                min_spread_pct: None,
                anchor_source: None,
            });
            entry.min_spread_pct = min_spread_pct;
            entry.anchor_source = anchor_source;
        }

        Ok(by_code
            .into_values()
            .map(|e| CurrencyTierOverride {
                currency: e.currency,
                tiers: <[TierMargin; 4]>::try_from(e.tiers).ok(),
                min_spread_pct: e.min_spread_pct,
                anchor_source: e.anchor_source,
            })
            .collect())
    }

    /// Inserts `overrides` on `policy_version_id`: four `rate_policy_currency_tiers` rows for each override with
    /// `tiers`, and one `rate_policy_currency_settings` row when it has a `min_spread_pct` or an `anchor_source`.
    pub async fn insert(
        tx: &mut Transaction<'_, Postgres>,
        policy_version_id: &Uuid,
        overrides: &[CurrencyTierOverride],
    ) -> Result<(), RateEngineError> {
        for ov in overrides {
            let code = ov.currency.as_str();
            if let Some(tiers) = &ov.tiers {
                for t in tiers {
                    sqlx::query(
                        "INSERT INTO rate_policy_currency_tiers \
                             (policy_version_id, currency_code, tier_no, buy_margin_pct, sell_margin_pct) \
                         VALUES ($1, $2, $3, $4, $5)",
                    )
                    .bind(policy_version_id)
                    .bind(code)
                    .bind(i16::from(t.tier_no))
                    .bind(t.buy_margin_pct.value())
                    .bind(t.sell_margin_pct.value())
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;
                }
            }
            if ov.min_spread_pct.is_some() || ov.anchor_source.is_some() {
                sqlx::query(
                    "INSERT INTO rate_policy_currency_settings \
                         (policy_version_id, currency_code, min_spread_pct, anchor_source) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(policy_version_id)
                .bind(code)
                .bind(ov.min_spread_pct.map(RatePercent::value))
                .bind(ov.anchor_source.map(AnchorSource::as_str))
                .execute(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Copies every override row (tiers and settings) of version `from` onto version `to`, except the currencies in
    /// `except` (§3 "Versioned like the rest of the policy"). `PolicyVersionWriter::next` inserts the replaced
    /// currencies first, then carries the others with `except` = those currencies, so a replaced currency is never
    /// copied and an override with no tiers, no minimum spread and no anchor removes that currency's override.
    pub async fn carry_forward(
        tx: &mut Transaction<'_, Postgres>,
        from: &Uuid,
        to: &Uuid,
        except: &[CurrencyCode],
    ) -> Result<(), RateEngineError> {
        let excluded: Vec<&str> = except.iter().map(|c| c.as_str()).collect();

        sqlx::query(
            "INSERT INTO rate_policy_currency_tiers \
                 (policy_version_id, currency_code, tier_no, buy_margin_pct, sell_margin_pct) \
             SELECT $2, currency_code, tier_no, buy_margin_pct, sell_margin_pct \
             FROM rate_policy_currency_tiers \
             WHERE policy_version_id = $1 AND NOT (currency_code = ANY($3))",
        )
        .bind(from)
        .bind(to)
        .bind(&excluded)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        sqlx::query(
            "INSERT INTO rate_policy_currency_settings \
                 (policy_version_id, currency_code, min_spread_pct, anchor_source) \
             SELECT $2, currency_code, min_spread_pct, anchor_source \
             FROM rate_policy_currency_settings \
             WHERE policy_version_id = $1 AND NOT (currency_code = ANY($3))",
        )
        .bind(from)
        .bind(to)
        .bind(&excluded)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        Ok(())
    }
}
