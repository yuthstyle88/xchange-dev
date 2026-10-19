//! B05 `StockReservation` (design `docs/design/blueprint/stock-reservation.md` §2 D-2, D-3, §3, §5).
//!
//! Skeleton by the B05 sub-controller (claude/6530d490) on `feat/b05-stock`: the §5 names with `todo!()` bodies.
//! SR-1 fills the bodies.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

/// `stock_reservations.id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub Uuid);

/// Why a reservation was released (`stock_reservations.released_reason`, §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// Past `expires_at`, set lazily by the next reserve of the same (branch, currency) (D-3).
    Expired,
    /// Its quote was cancelled (D-6).
    Cancelled,
}

impl ReleaseReason {
    /// The stored text: `EXPIRED` / `CANCELLED`.
    pub fn as_str(self) -> &'static str {
        todo!("SR-1")
    }

    /// The stored text back; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let _ = text;
        todo!("SR-1")
    }
}

/// The stored status (D-3): `RESERVED → CONSUMED | RELEASED`. "Active" is not a status: it is `Reserved` and
/// `expires_at > now` ([`StockReservation::is_active`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationStatus {
    Reserved,
    Consumed,
    Released(ReleaseReason),
}

impl ReservationStatus {
    /// The stored `status` text: `RESERVED` / `CONSUMED` / `RELEASED`.
    pub fn as_str(self) -> &'static str {
        todo!("SR-1")
    }
}

/// One row of `stock_reservations` (§3): one per quote line (D-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockReservation {
    pub id: ReservationId,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub quote_line_id: Uuid,
    pub currency: CurrencyCode,
    /// `None` = denomination group ALL (§19).
    pub denomination_id: Option<i64>,
    pub quantity: ForeignAmount,
    /// Its quote's `expires_at` (D-7).
    pub expires_at: DateTime<Utc>,
    pub status: ReservationStatus,
    /// Set when `Consumed`.
    pub transaction_id: Option<Uuid>,
}

impl StockReservation {
    /// D-3: `Reserved` and `expires_at > now`. A `Reserved` row at or past `expires_at` holds nothing.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        let _ = now;
        todo!("SR-1")
    }
}
