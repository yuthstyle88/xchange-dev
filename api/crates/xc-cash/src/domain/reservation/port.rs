//! B05 port `StockReservations` (design `docs/design/blueprint/stock-reservation.md` §4, §5).
//!
//! Skeleton by the B05 sub-controller (claude/6530d490). Shape vs §5's diagram (`[proposed]`, internal):
//! - `reserve_in` takes the quote line only: the store reads the line's quote, branch, currency, side, quantity and
//!   the quote's `expires_at` itself, so D-7 (expiry = the quote's) and D-15 (SELL lines of a foreign currency
//!   only) live in the core and no caller can get them wrong;
//! - every call takes `now`, so expiry (D-3) is testable;
//! - `availability_in` takes the organisation and reads on a plain connection (views read on the pool).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use super::availability::StockAvailability;
use super::stock_reservation::{ReleaseReason, ReservationId};

/// What a reserve names (D-2): one quote line. `denomination_id` `None` = group ALL (§19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReserveLine {
    pub quote_line_id: Uuid,
    pub denomination_id: Option<i64>,
}

pub trait StockReservations: Send + Sync {
    /// D-4, D-7, D-8, D-15, D-16. In the caller's transaction: locks the line's `inventory_positions` row
    /// `FOR UPDATE` (L-5 order), sets the (branch, currency)'s expired `RESERVED` rows `RELEASED`/`EXPIRED` (D-3),
    /// and inserts one `RESERVED` row with the quote's `expires_at` only if `available ≥ quantity`; else
    /// `stockInsufficient` and nothing written. A BUY line or a THB line is refused `validationFailed` (D-15).
    /// The same line again while its row is live answers that row's id and writes nothing (D-8). No approval
    /// argument exists: a reservation never exceeds `available` (D-16).
    fn reserve_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        line: &'a ReserveLine,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<ReservationId>>;

    /// D-5. Sets the quote's `RESERVED` rows `CONSUMED` with `transaction_id`. A row past `expires_at` is
    /// re-reserved first under the same lock: consumed if `available ≥ quantity`, else `stockInsufficient`
    /// (the caller rolls back). A quote with no `RESERVED` row is `Ok(())` and writes nothing (S12).
    fn consume_for_quote_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote_id: Uuid,
        transaction_id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// D-6. Sets the quote's `RESERVED` rows `RELEASED` with `reason`. None is `Ok(())`.
    fn release_for_quote_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote_id: Uuid,
        reason: ReleaseReason,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// D-1. `on_hand` = the position's quantity (0 with no row); `reserved` = Σ of the active rows (D-3).
    fn availability_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: Uuid,
        branch: &'a BranchId,
        currency: CurrencyCode,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<StockAvailability>>;
}
