//! Resolution (pricing-formula.md §3): the policy version's own formula → the organisation's current formula → the
//! built-in default.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::formula::price_formula::PriceFormula;

/// Which formula priced a line (`rate_draft_lines.formula_source`, `pricing_tier_sets.formula_source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormulaSource {
    /// The branch's policy version (`rate_policy_versions.id`) carries its own formula.
    Branch(Uuid),
    /// The organisation's formula version (`pricing_formula_versions.id`).
    Organisation(Uuid),
    Default,
}

impl FormulaSource {
    /// `BRANCH` · `ORGANISATION` · `DEFAULT` (the column's CHECK values, §8).
    pub fn as_db(&self) -> &'static str {
        match self {
            FormulaSource::Branch(_) => "BRANCH",
            FormulaSource::Organisation(_) => "ORGANISATION",
            FormulaSource::Default => "DEFAULT",
        }
    }

    /// The `formula_version_id` column: the organisation formula's id, else `None`.
    pub fn formula_version_id(&self) -> Option<Uuid> {
        match self {
            FormulaSource::Organisation(id) => Some(*id),
            _ => None,
        }
    }
}

/// The organisation's current formula version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganisationFormula {
    pub id: Uuid,
    pub version: i32,
    pub formula: PriceFormula,
}

pub struct FormulaResolver;

impl FormulaResolver {
    /// The §3 order, pure: `branch` = (policy version id, its formula) when the version carries one; `organisation` =
    /// (formula version id, formula) when the organisation has a current one.
    pub fn pick(
        branch: Option<(Uuid, PriceFormula)>,
        organisation: Option<(Uuid, PriceFormula)>,
    ) -> (PriceFormula, FormulaSource) {
        let _ = (branch, organisation);
        todo!("pricing-formula wave 2: FormulaResolver::pick")
    }

    /// The organisation's row of `pricing_formula_versions` with `valid_to IS NULL`, if any. Stored text that does
    /// not parse ⇒ `RateEngineError::Internal`.
    pub async fn organisation_current(
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> Result<Option<OrganisationFormula>, RateEngineError> {
        let _ = (tx, organization_id);
        todo!("pricing-formula wave 2: FormulaResolver::organisation_current")
    }

    /// Reads `rate_policy_versions.buy_formula` / `sell_formula` of `policy_version` (none ⇒ no branch formula), the
    /// branch's `organization_id` (`branches`), then `organisation_current`, and answers `pick`.
    pub async fn resolve(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        policy_version: Option<Uuid>,
    ) -> Result<(PriceFormula, FormulaSource), RateEngineError> {
        let _ = (tx, branch, policy_version);
        todo!("pricing-formula wave 2: FormulaResolver::resolve")
    }
}
