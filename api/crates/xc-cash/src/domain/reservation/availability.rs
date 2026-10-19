//! B05 `StockAvailability` (design `docs/design/blueprint/stock-reservation.md` D-1, §5).
//!
//! Skeleton by the B05 sub-controller (claude/6530d490); SR-1 fills the body.

use rust_decimal::Decimal;

/// One (branch, currency)'s figures (D-1): `on_hand` is today's figure (the position's quantity), `reserved` is
/// Σ `quantity` of the branch's **active** reservations of that currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockAvailability {
    pub on_hand: Decimal,
    pub reserved: Decimal,
}

impl StockAvailability {
    /// D-1: `on_hand − reserved`. It may be below zero when an approved posting (N-1) took the position below zero.
    pub fn available(&self) -> Decimal {
        todo!("SR-1")
    }
}
