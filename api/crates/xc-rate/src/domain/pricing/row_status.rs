//! A pricing board row's status (pricing-tiers.md §5.1, V.9's `STATUS`).
//!
//! SKELETON (PT-0): `RowStatus::of` is PT-8's for the rows §11 fixes (no anchor ⇒ `Manual`); its other rows wait for
//! V.9's `runtime/pricing.mjs` (owner P-2, §14 Q-P3). The type and signature stay.

use crate::domain::pricing::pricing_suggestion::Suggestion;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowStatus {
    Current,
    Pending,
    Unpriced,
    Tiers,
    Review,
    Manual,
    Reference,
}

impl RowStatus {
    /// The row's status from its suggestion, whether its current prices were approved, and whether the currency is
    /// enabled at the branch.
    pub fn of(_suggestion: &Suggestion, _approved: bool, _enabled: bool) -> RowStatus {
        todo!("PT-8: RowStatus::of")
    }
}
