//! `SlotRepository` (design §5): the slot's capacity, changed only inside the caller's transaction.

use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::booking_slot::BookingSlot;
use super::slot_error::SlotError;

/// What a move did (B4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotMove {
    /// `from == to`: nothing changed.
    Unchanged,
    /// One place left `from` and one was taken in `to`; both rows as updated.
    Moved { from: BookingSlot, to: BookingSlot },
}

pub trait SlotRepository: Send + Sync {
    /// B3: one `UPDATE booking_slots SET reserved = reserved + 1 WHERE id = $1 AND status = 'OPEN' AND
    /// reserved < capacity RETURNING …`; no row back = `CapacityFull`.
    fn reserve_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        slot: Uuid,
    ) -> BoxFuture<'a, Result<BookingSlot, SlotError>>;

    /// Gives one place back (`reserved - 1`); `StaleVersion` when none is held.
    fn release_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        slot: Uuid,
    ) -> BoxFuture<'a, Result<BookingSlot, SlotError>>;

    /// B4: lock both rows in ascending `id` order, reserve `to` (B3), release `from`. `from == to` is `Unchanged`.
    fn move_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        from: Uuid,
        to: Uuid,
    ) -> BoxFuture<'a, Result<SlotMove, SlotError>>;
}
