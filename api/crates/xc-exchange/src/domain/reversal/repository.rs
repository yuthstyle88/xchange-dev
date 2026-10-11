//! The reversal store (design §6): the only writer of `reversals`. Every method works on the caller's transaction.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use uuid::Uuid;

use super::reversal::{NewReversal, PostedReversalRow, Reversal, ReversalId, ReversalStatus};
use crate::domain::transaction::transaction::TransactionId;

pub trait ReversalRepository: Send + Sync {
    /// Inserts a REQUESTED reversal. A second live reversal of the same exchange is refused
    /// (`one_live_reversal_per_transaction`) with `staleVersion` naming the live one.
    fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        reversal: &'a NewReversal,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    /// Ties a REQUESTED reversal to its AP-1 request (RG6).
    fn set_approval_request_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
        request: Uuid,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
    ) -> impl std::future::Future<Output = AppResult<Option<Reversal>>> + Send + 'a;

    /// The live (not REJECTED) reversal of an exchange, if any (V-5).
    fn live_of_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        original: TransactionId,
    ) -> impl std::future::Future<Output = AppResult<Option<ReversalId>>> + Send + 'a;

    /// Moves `from → to`; `staleVersion` when the stored status is not `from`.
    fn set_status_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
        from: ReversalStatus,
        to: ReversalStatus,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    /// APPROVED → POSTED with what posting wrote (RV-3); `staleVersion` when not APPROVED.
    fn post_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: ReversalId,
        posted: &'a PostedReversalRow,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;
}
