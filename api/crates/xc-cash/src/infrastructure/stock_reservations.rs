//! B05 `PgStockReservations` (design `docs/design/blueprint/stock-reservation.md` §3, §4, D-1 … D-8, D-15, D-16).
//!
//! Skeleton by the B05 sub-controller (claude/6530d490) on `feat/b05-stock`; SR-1 fills the bodies.
//! `new()` takes no pool: every statement runs on the caller's transaction or connection, as
//! `LedgerCashPostings` does.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::reservation::availability::StockAvailability;
use crate::domain::reservation::port::{ReserveLine, StockReservations};
use crate::domain::reservation::stock_reservation::{ReleaseReason, ReservationId};

#[derive(Clone, Default)]
pub struct PgStockReservations;

impl PgStockReservations {
    pub fn new() -> Self {
        Self
    }
}

/// Σ `quantity` of the active reservations (D-3: `RESERVED` and `expires_at > now`) of one (organisation, branch,
/// currency). The one reading of "reserved": `availability_in`, the `branch_stock` view (SR-4), the low-stock alert
/// (SR-5) and the completion's check in `LedgerCashPostings` (SR-6) all call this.
pub async fn active_reserved_in(
    conn: &mut PgConnection,
    organization_id: Uuid,
    branch: &BranchId,
    currency: CurrencyCode,
    now: DateTime<Utc>,
) -> AppResult<Decimal> {
    let _ = (conn, organization_id, branch, currency, now);
    todo!("SR-1")
}

impl StockReservations for PgStockReservations {
    fn reserve_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        line: &'a ReserveLine,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<ReservationId>> {
        let _ = (tx, line, now);
        todo!("SR-1")
    }

    fn consume_for_quote_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote_id: Uuid,
        transaction_id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        let _ = (tx, quote_id, transaction_id, now);
        todo!("SR-1")
    }

    fn release_for_quote_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote_id: Uuid,
        reason: ReleaseReason,
    ) -> BoxFuture<'a, AppResult<()>> {
        let _ = (tx, quote_id, reason);
        todo!("SR-1")
    }

    fn availability_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: Uuid,
        branch: &'a BranchId,
        currency: CurrencyCode,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<StockAvailability>> {
        let _ = (conn, organization_id, branch, currency, now);
        todo!("SR-1")
    }
}
