//! FOLLOW or MANUAL per currency, MAIN only (pricing-tiers.md §15.2 `pricing_modes`, §15.5; `H:` §6: absence = Manual).
//!
//! The wire strings are the contract and are written in the skeleton (XC-V9-PRICING).

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use xc_kernel::money::CurrencyCode;

use crate::domain::pricing::tier_set::StoreFuture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PricingMode {
    Follow,
    Manual,
}

impl PricingMode {
    /// `"FOLLOW"` · `"MANUAL"`.
    pub fn as_str(self) -> &'static str {
        match self {
            PricingMode::Follow => "FOLLOW",
            PricingMode::Manual => "MANUAL",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "FOLLOW" => Some(PricingMode::Follow),
            "MANUAL" => Some(PricingMode::Manual),
            _ => None,
        }
    }
}

/// One `pricing_modes` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingModeRow {
    pub currency: CurrencyCode,
    pub mode: PricingMode,
    pub revision: i64,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

/// `pricing_modes`. The Pg impl is `infrastructure::pricing::pricing_mode_store`.
pub trait PricingModeStore: Send + Sync {
    /// Upserts the currency's mode and bumps `revision` (1 for a new row). Returns the new revision.
    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
        mode: PricingMode,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, i64>;

    fn get_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingModeRow>>;

    /// Every currency whose mode is `FOLLOW`, by code.
    fn following_in<'a>(&'a self, conn: &'a mut PgConnection)
    -> StoreFuture<'a, Vec<CurrencyCode>>;

    /// `max(revision)` over every row, for `pricing_board`'s version (§15.9).
    fn newest_revision_in<'a>(&'a self, conn: &'a mut PgConnection)
    -> StoreFuture<'a, Option<i64>>;
}
