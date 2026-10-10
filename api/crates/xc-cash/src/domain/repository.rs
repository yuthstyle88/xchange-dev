//! Cash repository port.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::count::{CashCount, CountLine};

#[derive(Debug, Clone)]
pub struct CountTotal {
    pub currency: String,
    pub counted: Decimal,
    pub expected: Option<Decimal>,
    pub difference: Option<Decimal>,
}

pub trait CashRepository: Send + Sync {
    fn find_count_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CashCount>>>;

    fn find_count_by_idempotency_key<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CashCount>>>;

    fn insert_count<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count: &'a CashCount,
        lines: &'a [CountLine],
        totals: &'a [CountTotal],
    ) -> BoxFuture<'a, AppResult<()>>;

    fn is_newest_count<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count: &'a CashCount,
    ) -> BoxFuture<'a, AppResult<bool>>;

    fn get_count_totals<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Vec<CountTotal>>>;

    fn update_count_verified<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count_id: &'a str,
        verified_by: &'a str,
        verified_at: DateTime<Utc>,
        shift_id: Option<&'a str>,
        totals: &'a [CountTotal],
    ) -> BoxFuture<'a, AppResult<()>>;
}
