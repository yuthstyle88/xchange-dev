//! Store of per-currency overrides on a policy version (pricing-tiers.md §3, §10.1): `rate_policy_currency_tiers`
//! and `rate_policy_currency_settings`. Rows are immutable once inserted; a new version gets them by `carry_forward`.
//!
//! SKELETON (PT-0): the bodies are PT-3's. The signatures stay.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;

pub struct CurrencyTierOverrideStore;

impl CurrencyTierOverrideStore {
    /// Every currency override of `policy_version_id`, ordered by currency code. A currency appears when it has a
    /// settings row or tier rows; its `tiers` are `Some` only with exactly four tier rows.
    pub async fn for_version(
        _tx: &mut Transaction<'_, Postgres>,
        _policy_version_id: &Uuid,
    ) -> Result<Vec<CurrencyTierOverride>, RateEngineError> {
        todo!("PT-3: CurrencyTierOverrideStore::for_version")
    }

    /// Inserts `overrides` on `policy_version_id`: four `rate_policy_currency_tiers` rows for each override with
    /// `tiers`, and one `rate_policy_currency_settings` row when it has a `min_spread_pct` or an `anchor_source`.
    pub async fn insert(
        _tx: &mut Transaction<'_, Postgres>,
        _policy_version_id: &Uuid,
        _overrides: &[CurrencyTierOverride],
    ) -> Result<(), RateEngineError> {
        todo!("PT-3: CurrencyTierOverrideStore::insert")
    }

    /// Copies every override row of version `from` onto version `to` (§3 "Versioned like the rest of the policy").
    pub async fn carry_forward(
        _tx: &mut Transaction<'_, Postgres>,
        _from: &Uuid,
        _to: &Uuid,
    ) -> Result<(), RateEngineError> {
        todo!("PT-3: CurrencyTierOverrideStore::carry_forward")
    }
}
