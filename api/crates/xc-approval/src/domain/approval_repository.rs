//! `ApprovalRepository`: repository port for requests and decisions.

use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

use crate::domain::request::{ApprovalRequest, Outcome, RequestId};

/// Repository port for storing and querying approval requests and decisions.
pub trait ApprovalRepository: Send + Sync {
    /// Finds an existing request by type and idempotency key.
    fn find_by_type_and_key<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        type_name: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<ApprovalRequest>>>;

    /// Checks whether a request for `(type_name, object_id)` is currently PENDING.
    fn has_pending_for_object<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        type_name: &'a str,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;

    /// Inserts a new PENDING approval request.
    #[allow(clippy::too_many_arguments)]
    fn insert_request<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        type_name: &'a str,
        branch_id: Option<String>,
        object_id: &'a str,
        object_version_hash: &'a str,
        policy_version: &'a str,
        proposed_payload: Value,
        requested_by: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<ApprovalRequest>>;

    /// Fetches an approval request by id, locking the row with FOR UPDATE.
    fn find_by_id_for_update<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a RequestId,
    ) -> BoxFuture<'a, AppResult<Option<ApprovalRequest>>>;

    /// Updates the status of an approval request.
    fn update_status<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a RequestId,
        status: &'static str,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Inserts a decision row for an approval request.
    fn insert_decision<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        request_id: &'a RequestId,
        approver_id: &'a str,
        approver_role: &'a str,
        outcome: Outcome,
        reason: &'a str,
    ) -> BoxFuture<'a, AppResult<()>>;
}
