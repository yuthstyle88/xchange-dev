//! `CustomerMetricsSource` trait and metrics types (design §4.1, §6, §7).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberMetricsDto {
    #[serde(rename = "recencyDays", skip_serializing_if = "Option::is_none")]
    pub recency_days: Option<u32>,
    pub frequency: u32,
    #[serde(rename = "valueThb")]
    pub value_thb: String,
    #[serde(rename = "tierCode", skip_serializing_if = "Option::is_none")]
    pub tier_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerMetrics {
    pub customer_id: String,
    pub customer_name: String,
    pub recency_days: Option<u32>,
    pub frequency: u32,
    pub value_thb: Decimal,
    pub tier_code: Option<String>,
    pub home_branch: Option<String>,
}

pub trait CustomerMetricsSource: Send + Sync {
    fn load_candidate_metrics<'a>(
        &'a self,
        org: Uuid,
        lookback_months: u32,
        as_of: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<CustomerMetrics>>>;
}
