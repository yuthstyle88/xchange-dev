//! Flags on a pricing board row (pricing-tiers.md §5.1, §11).
//!
//! SKELETON (PT-0): `PricingFlag::blocking` is PT-8's. The type and signature stay.

/// Why a row needs a person's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PricingFlag {
    /// No market quote to anchor a suggestion on.
    NoAnchor,
    /// No TT quote; the suggestion is anchored on Super Rich.
    NoTtQuote,
    /// The anchoring source is older than the threshold.
    StaleSource,
    /// A tier's spread is below the currency's minimum spread.
    MinSpread,
    /// A suggested buy is below the partner cost.
    BelowCost,
    /// Tier 1 moves more than 1 % from the current publication (warns, does not block).
    LargeMove,
    /// The margin cannot reproduce the tier price at the board's display scale.
    Precision,
}

impl PricingFlag {
    /// True for the flags that keep a row out of bulk approve (V.9: `BelowCost`, `StaleSource`, `Precision`; and
    /// `NoAnchor`, whose row has no suggestion to approve, §11).
    pub fn blocking(self) -> bool {
        matches!(
            self,
            Self::NoAnchor | Self::StaleSource | Self::BelowCost | Self::Precision
        )
    }
}
