//! LY-5 (#1408): the `my_membership` view (design `membership-points.md` §4.6, C05):
//! a customer's own tier and live points.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_membership.ts")]
pub struct PointsDto {
    #[ts(type = "number")]
    pub available: i64,
    #[ts(type = "number")]
    pub pending: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_membership.ts")]
pub struct NextExpiryDto {
    #[ts(type = "number")]
    pub points: i64,
    #[ts(type = "string")]
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_membership.ts")]
pub struct TierProgressDto {
    pub basis: String,
    #[ts(type = "string")]
    pub current: Decimal,
    #[ts(type = "string | null")]
    pub next: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_membership.ts")]
pub struct MyMembershipData {
    pub tier: Option<String>,
    #[ts(type = "string | null")]
    pub effective_until: Option<DateTime<Utc>>,
    pub points: Option<PointsDto>,
    pub points_unavailable: bool,
    pub next_expiry: Option<NextExpiryDto>,
    pub progress: Option<TierProgressDto>,
}
