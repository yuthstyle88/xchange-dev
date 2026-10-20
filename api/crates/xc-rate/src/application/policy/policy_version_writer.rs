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

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::policy::anchor_source::AnchorSource;
use crate::domain::policy::currency_tier_override::CurrencyTierOverride;
use crate::domain::policy::policy_tiers::PolicyTier;

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
    /// `None` carries every override of the current version forward (`CurrencyTierOverrideStore::carry_forward`);
    /// `Some(list)` replaces the overrides of the currencies in `list` and carries the others forward (§3).
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
        _tx: &mut Transaction<'_, Postgres>,
        _branch: &BranchId,
        _change: PolicyVersionChange,
    ) -> Result<WrittenVersion, RateEngineError> {
        todo!("PT-1: PolicyVersionWriter::next")
    }
}
