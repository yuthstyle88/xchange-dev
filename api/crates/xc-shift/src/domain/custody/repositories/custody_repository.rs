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

    fn store_state_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        state: &'a CustodyState,
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

    fn is_enabled_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>>;
}
