//! An approved or followed tier set (pricing-tiers.md §15.1, §15.2; V9:store `pricing_tiers`): four tier prices at the
//! currency's `unit_size` that **are** the counter price (V-1). Append-only: the latest row per (branch, currency) wins.
//!
//! SKELETON (XC-V9-PRICING): `TierRate::to_check` is PV-a1's. The wire strings are the contract and are written here.
//! Types and signatures stay.
//!
//! §15.2a names the per-tier pair `TierPrice`; that name is already `pricing_suggestion::TierPrice` (whose sides are
//! `Option`, for a suggestion with no anchor), so the approved pair is `TierRate`: both sides present.

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::pricing_suggestion::TierPrice;

/// One approved tier, at the currency's `unit_size`: `buy > 0`, `sell > buy` (§15.2 `tiers`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierRate {
    pub buy: Rate,
    pub sell: Rate,
}

impl TierRate {
    /// The four tiers in `TierCheck::validate`'s shape (`tier_no` 1..=4, both sides `Some`).
    pub fn to_check(tiers: &[TierRate; 4]) -> [TierPrice; 4] {
        let _ = (tiers,);
        todo!("PV-a1: TierRate::to_check")
    }
}

/// Who wrote a tier set (§15.2 `source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TierSetSource {
    /// An approve at the branch itself (§15.3).
    Manual,
    /// MAIN's approve adjusted into a branch (§15.4).
    MainPush,
    /// The Follow publisher, at MAIN or at a followed branch (§15.5).
    Follow,
}

impl TierSetSource {
    /// `"MANUAL"` · `"MAIN_PUSH"` · `"FOLLOW"`.
    pub fn as_str(self) -> &'static str {
        match self {
            TierSetSource::Manual => "MANUAL",
            TierSetSource::MainPush => "MAIN_PUSH",
            TierSetSource::Follow => "FOLLOW",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "MANUAL" => Some(TierSetSource::Manual),
            "MAIN_PUSH" => Some(TierSetSource::MainPush),
            "FOLLOW" => Some(TierSetSource::Follow),
            _ => None,
        }
    }

    /// What the board and the public projection call it: `MAIN_PUSH` reads as `MANUAL` (§15.7).
    pub fn pricing_source(self) -> PricingSource {
        match self {
            TierSetSource::Manual | TierSetSource::MainPush => PricingSource::Manual,
            TierSetSource::Follow => PricingSource::Follow,
        }
    }
}

/// The effective pricing mode of a line or a (branch, currency) (§15.1, §15.7 `pricingSource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PricingSource {
    Auto,
    Manual,
    Follow,
}

impl PricingSource {
    /// `"AUTO"` · `"MANUAL"` · `"FOLLOW"`.
    pub fn as_str(self) -> &'static str {
        match self {
            PricingSource::Auto => "AUTO",
            PricingSource::Manual => "MANUAL",
            PricingSource::Follow => "FOLLOW",
        }
    }
}

/// The market quote a tier set was checked against (§15.2 `anchor`), per one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketAnchor {
    pub source: MarketSource,
    pub buy: Rate,
    pub sell: Rate,
    pub fetched_at: DateTime<Utc>,
    /// The `reference_batches.id` the quote came from (FOLLOW's `request_id` is keyed on it, §15.5).
    pub batch_id: Option<Uuid>,
}

/// One `pricing_tier_sets` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingTierSet {
    pub id: Uuid,
    /// Idempotency, UNIQUE: an approve writes `<Idempotency-Key>:<currency>` (one key may approve several currencies);
    /// a push writes `<MAIN row's request_id>:<branch>`; FOLLOW at MAIN writes `follow:MAIN:<code>:<anchor batch id>`.
    pub request_id: String,
    pub branch: String,
    pub currency: CurrencyCode,
    pub source: TierSetSource,
    /// MAIN's set a branch row was adjusted from (`MainPush`, and `Follow` at a followed branch).
    pub base_tier_set_id: Option<Uuid>,
    pub tiers: [TierRate; 4],
    pub anchor: Option<MarketAnchor>,
    /// `PricingFlag` names the suggestion raised (kept for the record; not a gate).
    pub flags: Vec<String>,
    pub draft_id: Option<Uuid>,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
}

pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RateEngineError>> + Send + 'a>>;

/// `pricing_tier_sets` (§15.2a). The Pg impl is `infrastructure::pricing::tier_set_store::PgTierSetStore`.
pub trait TierSetStore: Send + Sync {
    /// Inserts the row. `false` when a row with the same `request_id` already exists (nothing written).
    fn insert_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        set: &'a PricingTierSet,
    ) -> StoreFuture<'a, bool>;

    /// The latest row for (branch, currency), any source.
    fn latest_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingTierSet>>;

    /// The latest row for (branch, currency) with `source = 'FOLLOW'` (§15.7 `MISMATCH`).
    fn latest_follow_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        currency: CurrencyCode,
    ) -> StoreFuture<'a, Option<PricingTierSet>>;

    /// One row by id.
    fn get_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        id: Uuid,
    ) -> StoreFuture<'a, Option<PricingTierSet>>;

    /// Records the draft that publishes the set (§15.3 step 4 runs after the row is written). `PublishService` finds
    /// the set behind a draft line through `draft_id` to write `source_snapshot.pricing`.
    fn set_draft_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        id: Uuid,
        draft_id: Uuid,
    ) -> StoreFuture<'a, ()>;

    /// One row by `request_id` (a retried push or follow finds what it wrote).
    fn by_request_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        request_id: &'a str,
    ) -> StoreFuture<'a, Option<PricingTierSet>>;
}
