//! `PgSlotRepository` (design §5): [`SlotRepository`] on `booking_slots`.

use chrono::{DateTime, NaiveDate, Utc};
use futures::future::BoxFuture;
use pos_error::AppError;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use crate::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use crate::domain::slot::repository::{SlotMove, SlotRepository};
use crate::domain::slot::slot_error::SlotError;

pub struct PgSlotRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgSlotRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type SlotDbRow = (
    Uuid,
    Uuid,
    String,
    NaiveDate,
    DateTime<Utc>,
    DateTime<Utc>,
    i32,
    i32,
    String,
);

fn to_booking_slot(row: SlotDbRow) -> BookingSlot {
    let (id, org_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) = row;
    let status = if status == "CLOSED" {
        SlotStatus::Closed
    } else {
        SlotStatus::Open
    };
    BookingSlot {
        id,
        organization_id: org_id,
        branch_id: BranchId::new(branch_id),
        service_date,
        start_at,
        end_at,
        capacity,
        reserved,
        status,
    }
}

impl SlotRepository for PgSlotRepository {
    fn reserve_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        slot: Uuid,
    ) -> BoxFuture<'a, Result<BookingSlot, SlotError>> {
        Box::pin(async move {
            let row: Option<SlotDbRow> = sqlx::query_as(
                "UPDATE booking_slots \
                 SET reserved = reserved + 1, updated_at = now() \
                 WHERE id = $1 AND status = 'OPEN' AND reserved < capacity \
                 RETURNING id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status",
            )
            .bind(slot)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

            match row {
                Some(r) => Ok(to_booking_slot(r)),
                None => Err(SlotError::CapacityFull),
            }
        })
    }

    fn release_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        slot: Uuid,
    ) -> BoxFuture<'a, Result<BookingSlot, SlotError>> {
        Box::pin(async move {
            let row: Option<SlotDbRow> = sqlx::query_as(
                "UPDATE booking_slots \
                 SET reserved = reserved - 1, updated_at = now() \
                 WHERE id = $1 AND reserved > 0 \
                 RETURNING id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status",
            )
            .bind(slot)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

            match row {
                Some(r) => Ok(to_booking_slot(r)),
                None => Err(SlotError::StaleVersion),
            }
        })
    }

    fn move_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        from: Uuid,
        to: Uuid,
    ) -> BoxFuture<'a, Result<SlotMove, SlotError>> {
        Box::pin(async move {
            if from == to {
                return Ok(SlotMove::Unchanged);
            }

            let (first, second) = if from < to { (from, to) } else { (to, from) };

            let row_first: Option<(Uuid,)> =
                sqlx::query_as("SELECT id FROM booking_slots WHERE id = $1 FOR UPDATE")
                    .bind(first)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| SlotError::Store(AppError::Database(e)))?;

            let row_second: Option<(Uuid,)> =
                sqlx::query_as("SELECT id FROM booking_slots WHERE id = $1 FOR UPDATE")
                    .bind(second)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| SlotError::Store(AppError::Database(e)))?;

            if row_first.is_none() || row_second.is_none() {
                return Err(SlotError::NotFound);
            }

            let to_slot = self.reserve_in(tx, to).await?;
            let from_slot = self.release_in(tx, from).await?;

            Ok(SlotMove::Moved {
                from: from_slot,
                to: to_slot,
            })
        })
    }
}
