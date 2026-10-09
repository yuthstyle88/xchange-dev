//! Shift repository trait.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::shift::entities::shift::Shift;
use crate::domain::shift::entities::stock_movement::StockMovement;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::StockDelta;

pub trait ShiftRepository: Send + Sync + 'static {
    fn branch_exists(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<bool>> + Send;

    fn active(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<Option<Shift>>> + Send;

    fn shifts(
        &self,
        branch: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<Vec<Shift>>> + Send;

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

    fn active_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
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
