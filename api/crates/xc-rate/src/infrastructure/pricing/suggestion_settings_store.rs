//! `PricingSuggestionSettingsStore`: `pricing_suggestion_settings` (pricing-tiers.md §5.2, §12.1).
//!
//! SKELETON (XC-PRICING PX-0): every body is PX-10's. Signatures stay.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgConnection;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::suggestion_settings::SuggestionSettings;

pub struct PricingSuggestionSettingsStore;

impl PricingSuggestionSettingsStore {
    /// The branch's `'*'` row and its currency rows.
    pub async fn for_branch(
        conn: &mut PgConnection,
        branch: &str,
    ) -> Result<SuggestionSettings, RateEngineError> {
        let _ = (conn, branch);
        todo!("PX-10: PricingSuggestionSettingsStore::for_branch")
    }

    /// `scope` is `"*"` or a currency code. `Some(better)` upserts the row; `None` deletes it.
    pub async fn save(
        conn: &mut PgConnection,
        branch: &str,
        scope: &str,
        better: Option<[Decimal; 4]>,
        by: &str,
        at: DateTime<Utc>,
    ) -> Result<(), RateEngineError> {
        let _ = (conn, branch, scope, better, by, at);
        todo!("PX-10: PricingSuggestionSettingsStore::save")
    }
}
