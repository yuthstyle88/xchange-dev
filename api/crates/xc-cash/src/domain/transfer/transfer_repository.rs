//! `TransferRepository` port for transfer persistence.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::Actor;

use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferStatus,
};
use crate::domain::transfer::transfer_policy::TransferPolicy;

pub trait TransferRepository: Send + Sync {
    /// Loads a transfer and its lines by id.
    fn find_by_id<'a>(&'a self, id: &'a str) -> BoxFuture<'a, AppResult<Option<CashTransfer>>>;

    /// Loads a transfer and its lines by id within an existing transaction.
    fn find_by_id_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CashTransfer>>>;

    /// Inserts a new transfer and its lines in DRAFT status.
    fn insert<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        cmd: &'a NewTransfer,
        who: &'a Actor,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<CashTransfer>>;

    /// Updates the approval request id on the transfer.
    fn set_approval_request_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        approval_request_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Updates status of the transfer.
    fn update_status<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        status: TransferStatus,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Marks the transfer as DISPATCHED.
    fn mark_dispatched<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        by: &'a Actor,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Updates received quantities on lines and marks transfer as RECEIVED.
    fn mark_received<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        by: &'a Actor,
        lines: &'a [NewTransferLine],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Marks the transfer as RECONCILED with note.
    fn mark_reconciled<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        by: &'a Actor,
        note: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Finds the newest policy version for an organisation.
    fn find_latest_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<TransferPolicy>>>;
}
