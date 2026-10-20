//! Shift repository trait.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::shift::entities::open_shift_row::OpenShiftRow;
use crate::domain::shift::entities::shift::Shift;
use crate::domain::shift::entities::stock_movement::StockMovement;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::StockDelta;

pub trait ShiftRepository: Send + Sync + 'static {
    fn branch_exists(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<bool>> + Send;

    fn shifts(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<Vec<Shift>>> + Send;

    /// The teller's OPEN shift on any branch (CA-10p, design §4.7), keyed by `shifts.teller_id`.
    /// Two OPEN shifts for one teller is an `AppError::Internal` naming the teller.
    fn open_of_teller(
        &self,
        teller: &str,
    ) -> impl std::future::Future<Output = AppResult<Option<Shift>>> + Send;

    /// [`ShiftRepository::open_of_teller`] in the caller's transaction.
    fn open_of_teller_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        teller: &'a str,
    ) -> impl std::future::Future<Output = AppResult<Option<Shift>>> + Send + 'a;

    /// Every OPEN shift of the branch with its teller, by `opened_at` then shift id (CA-10p, §4.7).
    /// An OPEN shift with no teller, or a teller that is no user, is an `AppError::Internal`.
    fn open_of_branch(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<Vec<OpenShiftRow>>> + Send;

    fn movements(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<Vec<StockMovement>>> + Send;

    fn shift(
        &self,
        branch: &BranchId,
        id: &str,
    ) -> impl std::future::Future<Output = AppResult<Option<Shift>>> + Send;

    fn shift_movements(
        &self,
        branch: &BranchId,
        shift_id: &str,
    ) -> impl std::future::Future<Output = AppResult<Vec<StockMovement>>> + Send;

    fn is_request_abandoned<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> impl std::future::Future<Output = AppResult<bool>> + Send + 'a;

    fn find_shift_by_open_request_id<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> impl std::future::Future<Output = AppResult<Option<Shift>>> + Send + 'a;

    fn find_shift_by_close_request_id<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> impl std::future::Future<Output = AppResult<Option<Shift>>> + Send + 'a;

    fn insert_shift<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift: &'a Shift,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    fn find_movement_by_request_id<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> impl std::future::Future<Output = AppResult<Option<StockMovement>>> + Send + 'a;

    fn insert_movement<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        movement: &'a StockMovement,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    fn update_expected<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift_id: &'a str,
        delta: &'a StockDelta,
        new_expected: &'a Balances,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    #[allow(clippy::too_many_arguments)]
    fn close_shift<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift_id: &'a str,
        closed_at: DateTime<Utc>,
        closed_by: &'a str,
        counted: &'a Balances,
        difference: &'a StockDelta,
        note: &'a str,
        close_request_id: &'a str,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;
}
