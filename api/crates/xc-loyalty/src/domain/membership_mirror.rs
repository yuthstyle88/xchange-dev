//! `MembershipMirror` (design §6, §4.1, §4.5): local mirror of member tiers.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::membership_feed::MembershipPage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirroredTier {
    pub tier_code: String,
    pub tier_rule_id: Uuid,
    pub tier_rule_version: i32,
    pub effective_at: DateTime<Utc>,
    pub effective_until: Option<DateTime<Utc>>,
}

pub trait MembershipMirror: Send + Sync {
    fn tier_of<'a>(
        &'a self,
        org: Uuid,
        customer: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<MirroredTier>>>;

    fn apply_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org: Uuid,
        page: &'a MembershipPage,
    ) -> BoxFuture<'a, AppResult<()>>;
}
