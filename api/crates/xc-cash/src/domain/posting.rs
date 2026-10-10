//! Posting port for cash movements.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};
use xc_shift::domain::shift::value_objects::balances::Balances;

pub type ShiftId = String;
pub type TransferId = String;
pub type SourceRef = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CashPlace {
    Drawer(ShiftId),
    Safe(BranchId),
    InTransit(TransferId),
    Outside,
}

#[derive(Debug, Clone)]
pub struct PostingRequest {
    pub from: CashPlace,
    pub to: CashPlace,
    pub currency: CurrencyCode,
    pub amount: Money,
    pub source: SourceRef,
}

pub trait CashPostings: Send + Sync {
    fn post_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        req: &'a [PostingRequest],
    ) -> BoxFuture<'a, AppResult<()>>;

    fn expected_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>>;
}
