//! Outgoing ports for the booking domain.

use futures::future::BoxFuture;
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, Side};

use crate::domain::booking::entities::booking_steps::LiveCustomer;
use crate::domain::booking::value_objects::booking_rules::BookingRules;

pub trait BookingPorts: Send + Sync {
    fn branch_exists<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<bool>>;

    fn live_customer<'a>(
        &'a self,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<LiveCustomer>>>;

    fn rules<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Option<BookingRules>>>;

    fn rules_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<BookingRules>>>;

    fn has_open_shift_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>>;

    fn counter_rate_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        code: &'a CurrencyCode,
        side: Side,
    ) -> BoxFuture<'a, AppResult<Option<Rate>>>;
}
