//! `ApprovalSubject`: the per-type subject trait.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};

use crate::domain::request::Outcome;

/// Error returned by `ApprovalSubject::*_or_unavailable` methods (LY-6 #1411, R1).
#[derive(Debug)]
pub enum SubjectError {
    DependencyUnavailable { code: &'static str, message: String },
    Failed(AppError),
}

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

    /// Like `version_hash`, but allows a subject to signal `SubjectError::DependencyUnavailable` (R1).
    fn version_hash_or_unavailable<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, Result<String, SubjectError>> {
        Box::pin(async move {
            self.version_hash(tx, object_id)
                .await
                .map_err(SubjectError::Failed)
        })
    }

    /// Like `on_decided`, but allows a subject to signal `SubjectError::DependencyUnavailable` (R1).
    fn on_decided_or_unavailable<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, Result<(), SubjectError>> {
        Box::pin(async move {
            self.on_decided(tx, object_id, outcome)
                .await
                .map_err(SubjectError::Failed)
        })
    }
}
