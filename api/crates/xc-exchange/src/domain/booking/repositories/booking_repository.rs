//! Booking repository trait.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Rate;

use crate::domain::booking::entities::booking::Booking;
use crate::domain::booking::entities::booking_steps::StoredRequest;

pub trait BookingRepository: Send + Sync {
    fn list<'a>(
        &'a self,
        branch: &'a BranchId,
        limit: i64,
    ) -> BoxFuture<'a, AppResult<Vec<Booking>>>;

    fn find<'a>(
        &'a self,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Booking>>>;

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Booking>>>;

    fn find_request_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<StoredRequest>>>;

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        booking: &'a Booking,
        request_id: &'a str,
        request_hash: &'a str,
    ) -> BoxFuture<'a, AppResult<()>>;

    #[allow(clippy::too_many_arguments)]
    fn confirm_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
        rate: Rate,
        expires_at: DateTime<Utc>,
        at: DateTime<Utc>,
        by: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;

    fn cancel_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;

    fn mark_used_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
        submission_id: &'a str,
        document_ref: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;
}
