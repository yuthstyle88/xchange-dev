//! Resolution (pricing-formula.md §3): the policy version's own formula → the organisation's current formula → the
//! built-in default.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::formula::price_formula::PriceFormula;
use crate::domain::formula::text::FormulaText;

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

fn parse_pair(buy: &str, sell: &str) -> Result<PriceFormula, RateEngineError> {
    let buy = FormulaText::parse(buy).map_err(|e| RateEngineError::Internal(e.to_string()))?;
    let sell = FormulaText::parse(sell).map_err(|e| RateEngineError::Internal(e.to_string()))?;
    Ok(PriceFormula::new(buy, sell))
}

pub struct FormulaResolver;

impl FormulaResolver {
    /// The §3 order, pure: `branch` = (policy version id, its formula) when the version carries one; `organisation` =
    /// (formula version id, formula) when the organisation has a current one.
    pub fn pick(
        branch: Option<(Uuid, PriceFormula)>,
        organisation: Option<(Uuid, PriceFormula)>,
    ) -> (PriceFormula, FormulaSource) {
        match (branch, organisation) {
            (Some((id, formula)), _) => (formula, FormulaSource::Branch(id)),
            (None, Some((id, formula))) => (formula, FormulaSource::Organisation(id)),
            (None, None) => (PriceFormula::default(), FormulaSource::Default),
        }
    }

    /// The organisation's row of `pricing_formula_versions` with `valid_to IS NULL`, if any. Stored text that does
    /// not parse ⇒ `RateEngineError::Internal`.
    pub async fn organisation_current(
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> Result<Option<OrganisationFormula>, RateEngineError> {
        let row: Option<(Uuid, i32, String, String)> = sqlx::query_as(
            "SELECT id, version, buy_formula, sell_formula \
             FROM pricing_formula_versions \
             WHERE organization_id = $1 AND valid_to IS NULL",
        )
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;
        row.map(|(id, version, buy, sell)| {
            Ok(OrganisationFormula {
                id,
                version,
                formula: parse_pair(&buy, &sell)?,
            })
        })
        .transpose()
    }

    /// Reads `rate_policy_versions.buy_formula` / `sell_formula` of `policy_version` (none ⇒ no branch formula), the
    /// branch's `organization_id` (`branches`), then `organisation_current`, and answers `pick`.
    pub async fn resolve(
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        policy_version: Option<Uuid>,
    ) -> Result<(PriceFormula, FormulaSource), RateEngineError> {
        let branch_formula = match policy_version {
            Some(pv_id) => {
                let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                    "SELECT buy_formula, sell_formula FROM rate_policy_versions WHERE id = $1",
                )
                .bind(pv_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
                match row {
                    Some((Some(buy), Some(sell))) => Some((pv_id, parse_pair(&buy, &sell)?)),
                    _ => None,
                }
            }
            None => None,
        };
        let org_id: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
        let organisation = match org_id {
            Some(org_id) => Self::organisation_current(tx, org_id)
                .await?
                .map(|o| (o.id, o.formula)),
            None => None,
        };
        Ok(Self::pick(branch_formula, organisation))
    }
}
