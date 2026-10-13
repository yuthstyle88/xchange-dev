//! Policy tiers store: loads value tiers for a policy version (rates.md §12.3, §12.10).

use rust_decimal::Decimal;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::money::{Money, RatePercent};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::policy_tiers::{PolicyTier, PolicyTiers};

pub struct PolicyTiersStore;

impl PolicyTiersStore {
    /// Loads the policy tiers for `policy_version_id`.
    ///
    /// Returns `Some(PolicyTiers)` only when the version exists and `value_tiers_on = true`.
    /// Returns `None` if the version does not exist, or if `value_tiers_on = false`
    /// (even if `rate_policy_value_tiers` rows exist).
    pub async fn for_version(
        tx: &mut Transaction<'_, Postgres>,
        policy_version_id: &Uuid,
    ) -> Result<Option<PolicyTiers>, RateEngineError> {
        let flag_row: Option<(bool,)> =
            sqlx::query_as("SELECT value_tiers_on FROM rate_policy_versions WHERE id = $1")
                .bind(policy_version_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

        match flag_row {
            Some((true,)) => (),
            _ => return Ok(None),
        }

        let rows = sqlx::query(
            "SELECT tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct \
             FROM rate_policy_value_tiers \
             WHERE policy_version_id = $1 \
             ORDER BY tier_no ASC",
        )
        .bind(policy_version_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut policy_tiers = Vec::with_capacity(rows.len());
        for row in rows {
            let tier_no: i16 = row.get("tier_no");
            let upper_dec: Option<Decimal> = row.get("upper_inclusive_thb");
            let buy_dec: Decimal = row.get("buy_margin_pct");
            let sell_dec: Decimal = row.get("sell_margin_pct");

            let upper = match upper_dec {
                Some(u) => Some(Money::parse(&format!("{u:.2}")).map_err(|e| {
                    RateEngineError::ValidationFailed(format!(
                        "invalid upper bound in tier {tier_no}: {e}"
                    ))
                })?),
                None => None,
            };

            let buy_margin = RatePercent::parse(&format!("{buy_dec:.4}")).map_err(|e| {
                RateEngineError::ValidationFailed(format!(
                    "invalid buy margin in tier {tier_no}: {e}"
                ))
            })?;
            let sell_margin = RatePercent::parse(&format!("{sell_dec:.4}")).map_err(|e| {
                RateEngineError::ValidationFailed(format!(
                    "invalid sell margin in tier {tier_no}: {e}"
                ))
            })?;

            policy_tiers.push(PolicyTier::new(
                tier_no as u8,
                upper,
                buy_margin,
                sell_margin,
            ));
        }

        let pt = PolicyTiers::new(policy_tiers)?;
        Ok(Some(pt))
    }
}
