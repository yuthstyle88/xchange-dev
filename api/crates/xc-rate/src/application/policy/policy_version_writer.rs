//! The one writer of rate policy versions (pricing-tiers.md §6, §10.1). `ratePolicySave`, `pricingConfigure` and
//! `pricingApprove` all write a version through it; never three copies of the version logic.
//!
//! SKELETON (PT-0): `PolicyVersionWriter::next` is PT-1's, a pure move of `ratePolicySave`'s inline version insert
//! (`api/src/presentation/http/v3/commands/rate_policy_save.rs`: the next number, closing the current version, the
//! insert, its value tiers), plus the §3 / §3.1 carry-forward. The types and signature stay.

use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::anchor_source::AnchorSource;
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;
use crate::domain::policy::policy_tiers::PolicyTier;
use crate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;

/// Everything a new policy version holds, already validated by the caller (bounds, guards, the frame rule).
#[derive(Debug, Clone)]
pub struct PolicyVersionChange {
    pub buy_adjust_pct: Decimal,
    pub sell_adjust_pct: Decimal,
    pub guards: Value,
    pub max_reference_age_hours: Option<i32>,
    pub on_stale: Option<String>,
    pub stale_limit_thb: Option<Decimal>,
    pub value_tiers_on: bool,
    pub sharp_move_alert_pct: Option<Decimal>,
    /// The branch's four value tiers (`rate_policy_value_tiers`); empty when the version has none.
    pub value_tiers: Vec<PolicyTier>,
    /// `None` keeps the current version's anchor (or `BOT` for the first version), §3.1.
    pub anchor_source: Option<AnchorSource>,
    /// `None` carries every override of the current version forward (`CurrencyTierOverrideStore::carry_forward`
    /// with no exception); `Some(list)` inserts `list` (`CurrencyTierOverrideStore::insert`) and carries the other
    /// currencies forward (`carry_forward` with `except` = the currencies of `list`), §3. An entry with no tiers, no
    /// minimum spread and no anchor therefore removes that currency's override.
    pub overrides: Option<Vec<CurrencyTierOverride>>,
    pub approved_by: String,
    pub reason: String,
}

/// The version that was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenVersion {
    pub id: Uuid,
    pub version: i32,
}

pub struct PolicyVersionWriter;

impl PolicyVersionWriter {
    /// Writes the branch's next policy version: number = max + 1, the current version's `valid_to` set to now, the
    /// new row with `source_version` = the current one, its value tiers, its anchor and its overrides.
    pub async fn next(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        change: PolicyVersionChange,
    ) -> Result<WrittenVersion, RateEngineError> {
        // 1. the current version = the branch's row with valid_to IS NULL (id, version, anchor_source), FOR UPDATE; may be none;
        let cur: Option<(Uuid, i32, String)> = sqlx::query_as(
            "SELECT id, version, anchor_source \
             FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_to IS NULL \
             FOR UPDATE",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // 2. next number = COALESCE(MAX(version), 0) + 1 for the branch
        let cur_max_ver: Option<(i32,)> = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) FROM rate_policy_versions WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let next_ver = cur_max_ver.map(|r| r.0).unwrap_or(0) + 1;

        // 3. UPDATE rate_policy_versions SET valid_to = now() WHERE branch_id = $1 AND valid_to IS NULL
        sqlx::query(
            "UPDATE rate_policy_versions SET valid_to = now() WHERE branch_id = $1 AND valid_to IS NULL",
        )
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // 4. INSERT today's columns plus anchor_source = change.anchor_source if Some, else the current version's anchor_source, else 'BOT';
        // source_version = the current version's number (NULL for the first); approved_by / reason from change;
        let id = Uuid::new_v4();
        let source_ver = cur.as_ref().map(|(_, v, _)| *v);
        let anchor_str = match change.anchor_source {
            Some(a) => a.as_str().to_string(),
            None => match &cur {
                Some((_, _, a)) => a.clone(),
                None => "BOT".to_string(),
            },
        };

        sqlx::query(
            "INSERT INTO rate_policy_versions \
             (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
              max_reference_age_hours, on_stale, stale_limit_thb, valid_from, \
              approved_by, reason, source_version, value_tiers_on, sharp_move_alert_pct, anchor_source) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), $10, $11, $12, $13, $14, $15)",
        )
        .bind(id)
        .bind(branch.as_str())
        .bind(next_ver)
        .bind(change.buy_adjust_pct)
        .bind(change.sell_adjust_pct)
        .bind(&change.guards)
        .bind(change.max_reference_age_hours)
        .bind(&change.on_stale)
        .bind(change.stale_limit_thb)
        .bind(&change.approved_by)
        .bind(&change.reason)
        .bind(source_ver)
        .bind(change.value_tiers_on)
        .bind(change.sharp_move_alert_pct)
        .bind(&anchor_str)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        // 5. the value tiers: upper_inclusive_thb as numeric(20,2) via try_minor -> Decimal::new(cents, 2); a failure => RateEngineError::ValidationFailed
        for tier in &change.value_tiers {
            let upper_dec: Option<Decimal> = match tier.upper_inclusive_thb {
                Some(m) => Some(m.try_minor().map(|cents| Decimal::new(cents, 2)).map_err(
                    |_| {
                        RateEngineError::ValidationFailed(format!(
                            "tier {} upper bound exceeds maximum allowed value",
                            tier.tier_no
                        ))
                    },
                )?),
                None => None,
            };
            sqlx::query(
                "INSERT INTO rate_policy_value_tiers \
                 (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(tier.tier_no as i16)
            .bind(upper_dec)
            .bind(tier.buy_margin_pct.value())
            .bind(tier.sell_margin_pct.value())
            .execute(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
        }

        // 6. overrides: Some(list) => CurrencyTierOverrideStore::insert(tx, new_id, &list), then (if there is a current version) carry_forward(tx, current_id, new_id, <the currencies of list>); None => carry_forward(tx, current_id, new_id, &[])
        match change.overrides {
            Some(list) => {
                CurrencyTierOverrideStore::insert(tx, &id, &list).await?;
                if let Some((cur_id, _, _)) = cur {
                    let replaced: Vec<CurrencyCode> = list.iter().map(|o| o.currency).collect();
                    CurrencyTierOverrideStore::carry_forward(tx, &cur_id, &id, &replaced).await?;
                }
            }
            None => {
                if let Some((cur_id, _, _)) = cur {
                    CurrencyTierOverrideStore::carry_forward(tx, &cur_id, &id, &[]).await?;
                }
            }
        }

        // 7. answer WrittenVersion { id, version }
        Ok(WrittenVersion {
            id,
            version: next_ver,
        })
    }
}
