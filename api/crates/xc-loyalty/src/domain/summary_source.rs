//! `LoyaltySummarySource` (design §4.5, §6, #1408 ruling Q1).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;

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
    /// Live member summary for the given E.164 phone.
    fn summary<'a>(&'a self, phone: &'a str) -> BoxFuture<'a, AppResult<LoyaltyMemberSummary>>;
}

/// Default implementation answering "unavailable" (design D-9, #1408 ruling Q3).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableLoyaltySummary;

impl LoyaltySummarySource for UnavailableLoyaltySummary {
    fn summary<'a>(&'a self, _phone: &'a str) -> BoxFuture<'a, AppResult<LoyaltyMemberSummary>> {
        Box::pin(async move { Err(AppError::Internal("loyalty summary unavailable".to_owned())) })
    }
}
