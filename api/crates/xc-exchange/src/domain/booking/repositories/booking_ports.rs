//! Outgoing ports for the booking domain.

use futures::future::BoxFuture;
use pos_error::AppResult;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use crate::domain::booking::entities::booking_steps::LiveCustomer;
use crate::domain::booking::value_objects::booking_rules::BookingRules;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterLine {
    pub line_id: Uuid,
    pub rate: Rate,
    pub unit_size: UnitSize,
}

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
        amount: Option<Money>,
    ) -> BoxFuture<'a, AppResult<Option<CounterLine>>>;
}
