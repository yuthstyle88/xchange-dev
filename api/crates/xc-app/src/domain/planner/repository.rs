//! The planner's storage port (design §5 `PlannerRepository`). [proposed]: the methods may change
//! shape; the services' signatures are what the up-front tests pin.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;
use crate::domain::app_session::SessionHash;
use crate::domain::owner::DraftOwner;
use crate::domain::planner::planner_draft::PlannerDraft;

pub trait PlannerRepository: Send + Sync {
    /// The draft, whoever owns it; `None` when there is no such row or it expired at or before `now`
    /// (P13: an expired draft is gone; `now` is the caller's clock, the one that set `expires_at`).
    fn get_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Option<PlannerDraft>, CustomerAppError>>;

    /// Every draft of `owner` not expired at `now`, newest first (P13).
    fn list<'a>(
        &'a self,
        owner: &'a DraftOwner,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Vec<PlannerDraft>, CustomerAppError>>;

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        draft: &'a PlannerDraft,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>>;

    /// Writes the plan, `version + 1` and `updated_at` when the stored version is `expected`;
    /// `false` when it is not. `expires_at` is never written here.
    fn update_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        draft: &'a PlannerDraft,
        expected: i32,
    ) -> BoxFuture<'a, Result<bool, CustomerAppError>>;

    fn delete_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>>;

    /// One statement: every draft of `session` becomes `customer_id`'s. Answers how many moved.
    fn claim_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        session: &'a SessionHash,
        customer_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<u64, CustomerAppError>>;
}
