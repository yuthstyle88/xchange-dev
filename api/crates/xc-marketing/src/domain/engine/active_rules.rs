//! `ActiveRules` source trait and `ActiveRule` struct (design §6, §7).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::benefit::rule::BenefitRule;

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveRule {
    pub rule: BenefitRule,
    pub stack_group: String,
    pub priority: i32,
    pub version_id: Uuid,
    pub version_created_at: DateTime<Utc>,
    pub segment_snapshot_id: Option<Uuid>,
}

pub trait ActiveRules: Send + Sync {
    fn active_for<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        branch: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ActiveRule>>>;
}
