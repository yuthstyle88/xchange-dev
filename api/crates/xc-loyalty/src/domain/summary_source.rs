//! `LoyaltySummarySource` (design §6, §4.5): live member summary from Loyalty.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextExpiry {
    pub points: i64,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierProgress {
    pub basis: String,
    pub current: Decimal,
    pub next: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryTier {
    pub code: String,
    pub effective_until: Option<DateTime<Utc>>,
    pub progress: Option<TierProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoyaltyMemberSummary {
    pub phone_key: String,
    pub available: i64,
    pub pending: i64,
    pub next_expiry: Option<NextExpiry>,
    pub tier: Option<SummaryTier>,
}

pub trait LoyaltySummarySource: Send + Sync {
    /// Fetch member summary by customer reference.
    fn summary(&self, customer_ref: Uuid) -> BoxFuture<'_, AppResult<LoyaltyMemberSummary>>;
}
