//! Records which formula priced a draft line or a tier set (pricing-formula.md §3 "Audit", §3a, §8). A file of its
//! own so that `RateDraftService`, `PublishService` and the FOLLOW loop share one writer, and no existing struct
//! (`PricingTierSet`, `DraftLine`) gains a field.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::application::formula::formula_resolver::FormulaSource;
use crate::domain::draft::rate_engine_error::RateEngineError;

pub struct FormulaRecord;

impl FormulaRecord {
    /// `UPDATE rate_draft_lines SET formula_source, formula_version_id WHERE id = line_id`.
    pub async fn draft_line(
        tx: &mut Transaction<'_, Postgres>,
        line_id: Uuid,
        source: FormulaSource,
    ) -> Result<(), RateEngineError> {
        let _ = (tx, line_id, source);
        todo!("pricing-formula wave 2: FormulaRecord::draft_line")
    }

    /// `UPDATE pricing_tier_sets SET formula_source, formula_version_id WHERE id = tier_set_id`.
    pub async fn tier_set(
        tx: &mut Transaction<'_, Postgres>,
        tier_set_id: Uuid,
        source: FormulaSource,
    ) -> Result<(), RateEngineError> {
        let _ = (tx, tier_set_id, source);
        todo!("pricing-formula wave 2: FormulaRecord::tier_set")
    }
}
