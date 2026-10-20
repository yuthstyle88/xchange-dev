//! Custody repository trait.

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::custody::entities::stored_event::StoredEvent;
use crate::domain::custody::state::CustodyState;

pub trait CustodyRepository: Send + Sync + 'static {
    fn branch_exists<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<bool>>;

    fn load_state<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<CustodyState>>;

    fn load_state_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<CustodyState>>;

    /// Stores what a command made of the branch's state: `before` is the state
    /// as loaded in this transaction, `after` what it became. Only what is new
    /// or differs is written.
    fn store_state_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        before: &'a CustodyState,
        after: &'a CustodyState,
    ) -> BoxFuture<'a, AppResult<()>>;

    fn append_events_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        events: &'a [StoredEvent],
    ) -> BoxFuture<'a, AppResult<()>>;

    fn list_events<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<StoredEvent>>>;
}
