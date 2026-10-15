//! `pg_booking_lifecycle` (design `docs/design/blueprint/booking-slots.md` §5, #1641, #1863):
//! the single factory that builds a production [`BookingLifecycle`] with both PostgreSQL
//! repositories (`PgBookingRepository` and `PgSlotRepository`).

use std::sync::Arc;

use sqlx::PgPool;

use crate::application::lifecycle::BookingLifecycle;
use crate::infrastructure::booking_repository::PgBookingRepository;
use crate::infrastructure::slot_repository::PgSlotRepository;

pub fn pg_booking_lifecycle(pool: PgPool) -> Arc<BookingLifecycle> {
    Arc::new(
        BookingLifecycle::new(Arc::new(PgBookingRepository::new(pool.clone())))
            .with_slots(Arc::new(PgSlotRepository::new(pool))),
    )
}
