//! `MembershipFeed` (design §6, §4.5): changes feed from Loyalty.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipChange {
    pub phone_key: String,
    pub customer_ref: Option<Uuid>,
    pub tier_code: String,
    pub tier_rule_id: Uuid,
    pub tier_rule_version: i32,
    pub effective_at: DateTime<Utc>,
    pub effective_until: Option<DateTime<Utc>>,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipPage {
    pub changed: bool,
    pub items: Vec<MembershipChange>,
    pub next: i64,
}

pub trait MembershipFeed: Send + Sync {
    fn changes_after(&self, seq: i64) -> BoxFuture<'_, AppResult<MembershipPage>>;
}
