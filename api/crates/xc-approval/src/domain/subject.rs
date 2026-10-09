//! `ApprovalSubject`: the per-type subject trait.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};

use crate::domain::request::Outcome;

/// Trait implemented by objects that require approval.
pub trait ApprovalSubject: Send + Sync {
    /// The unique name of this subject type (e.g. "RATE_PUBLICATION").
    fn type_name(&self) -> &'static str;

    /// The current version hash of the object identified by `object_id`.
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>>;

    /// Hook invoked when a decision is made, running in the caller's transaction.
    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>>;
}
