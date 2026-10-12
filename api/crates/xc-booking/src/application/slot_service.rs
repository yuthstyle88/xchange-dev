//! `SlotService` (design §5, §4.4 `slotRuleSet`, `closureAdd`, `slotCapacitySet`, `slotClose`): ADMIN's slot
//! settings and the generator's run.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppError;
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::branch_hours::BranchHours;

use crate::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use crate::domain::slot::branch_closure::BranchClosure;
use crate::domain::slot::repository::SlotRepository;
use crate::domain::slot::slot_error::SlotError;
use crate::domain::slot::slot_generator::SlotGenerator;
use crate::domain::slot::slot_rule::SlotRule;

/// What one [`SlotService::generate`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Generated {
    /// Slots inserted.
    pub added: u32,
    /// Empty future slots closed because they no longer fit.
    pub closed: u32,
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

type ExistingSlotRow = (Uuid, DateTime<Utc>, DateTime<Utc>, i32, i32, String);

struct ExistingSlotInfo {
    id: Uuid,
    end_at: DateTime<Utc>,
    capacity: i32,
    reserved: i32,
    status: String,
}

pub struct SlotService {
    pool: PgPool,
    #[allow(dead_code)]
    slots: Arc<dyn SlotRepository>,
}

impl SlotService {
    pub fn new(pool: PgPool, slots: Arc<dyn SlotRepository>) -> Self {
        Self { pool, slots }
    }

    /// Saves the branch's rule (insert or replace); `by` is the ADMIN's `users.id`.
    pub async fn set_rule(
        &self,
        branch: &BranchId,
        rule: &SlotRule,
        by: Uuid,
    ) -> Result<(), SlotError> {
        let res = sqlx::query(
            "INSERT INTO booking_slot_rules ( \
                 organization_id, branch_id, slot_minutes, capacity, horizon_days, updated_by, updated_at, created_at \
             ) \
             SELECT organization_id, id, $2, $3, $4, $5, now(), now() \
             FROM branches WHERE id = $1 \
             ON CONFLICT (branch_id) DO UPDATE SET \
                 slot_minutes = EXCLUDED.slot_minutes, \
                 capacity = EXCLUDED.capacity, \
                 horizon_days = EXCLUDED.horizon_days, \
                 updated_by = EXCLUDED.updated_by, \
                 updated_at = now()",
        )
        .bind(branch.as_str())
        .bind(rule.slot_minutes)
        .bind(rule.capacity)
        .bind(rule.horizon_days)
        .bind(by)
        .execute(&self.pool)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        if res.rows_affected() == 0 {
            return Err(SlotError::NotFound);
        }
        Ok(())
    }

    /// Adds a closure date of the branch; `by` is the ADMIN's `users.id`.
    pub async fn add_closure(
        &self,
        branch: &BranchId,
        closure: &BranchClosure,
        by: Uuid,
    ) -> Result<(), SlotError> {
        let res = sqlx::query(
            "INSERT INTO branch_closures ( \
                 organization_id, branch_id, closed_on, reason, created_by, created_at \
             ) \
             SELECT organization_id, id, $2, $3, $4, now() \
             FROM branches WHERE id = $1 \
             ON CONFLICT (branch_id, closed_on) DO UPDATE SET \
                 reason = EXCLUDED.reason, \
                 created_by = EXCLUDED.created_by, \
                 created_at = now()",
        )
        .bind(branch.as_str())
        .bind(closure.closed_on)
        .bind(&closure.reason)
        .bind(by)
        .execute(&self.pool)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        if res.rows_affected() == 0 {
            return Err(SlotError::NotFound);
        }
        Ok(())
    }

    /// §4.1: a capacity below the slot's `reserved` is `CapacityBelowReserved` and changes nothing.
    pub async fn set_capacity(&self, slot: Uuid, capacity: i32) -> Result<BookingSlot, SlotError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let current: Option<(i32,)> =
            sqlx::query_as("SELECT reserved FROM booking_slots WHERE id = $1 FOR UPDATE")
                .bind(slot)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let Some((reserved,)) = current else {
            return Err(SlotError::NotFound);
        };

        if capacity < reserved {
            return Err(SlotError::CapacityBelowReserved);
        }

        let row: SlotDbRow = sqlx::query_as(
            "UPDATE booking_slots \
             SET capacity = $2, updated_at = now() \
             WHERE id = $1 \
             RETURNING id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status",
        )
        .bind(slot)
        .bind(capacity)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        tx.commit()
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let (id, org_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) =
            row;
        let status = if status == "CLOSED" {
            SlotStatus::Closed
        } else {
            SlotStatus::Open
        };
        Ok(BookingSlot {
            id,
            organization_id: org_id,
            branch_id: BranchId::new(branch_id),
            service_date,
            start_at,
            end_at,
            capacity,
            reserved,
            status,
        })
    }

    /// §4.1: a slot with places taken is `SlotHasBookings`; an empty one becomes `CLOSED`.
    pub async fn close(&self, slot: Uuid) -> Result<BookingSlot, SlotError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let current: Option<(i32,)> =
            sqlx::query_as("SELECT reserved FROM booking_slots WHERE id = $1 FOR UPDATE")
                .bind(slot)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let Some((reserved,)) = current else {
            return Err(SlotError::NotFound);
        };

        if reserved > 0 {
            return Err(SlotError::SlotHasBookings);
        }

        let row: SlotDbRow = sqlx::query_as(
            "UPDATE booking_slots \
             SET status = 'CLOSED', updated_at = now() \
             WHERE id = $1 \
             RETURNING id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status",
        )
        .bind(slot)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        tx.commit()
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let (id, org_id, branch_id, service_date, start_at, end_at, capacity, reserved, _status) =
            row;
        Ok(BookingSlot {
            id,
            organization_id: org_id,
            branch_id: BranchId::new(branch_id),
            service_date,
            start_at,
            end_at,
            capacity,
            reserved,
            status: SlotStatus::Closed,
        })
    }

    /// B2, §4.1: runs [`crate::domain::slot::slot_generator::SlotGenerator::slots_for`] on the branch's rule, hours
    /// and closures; inserts the planned slots that are missing, closes the branch's empty future slots in
    /// `[from, from + days)` that are no longer planned, never deletes a slot and never touches one with places
    /// taken.
    pub async fn generate(
        &self,
        branch: &BranchId,
        from: NaiveDate,
        days: u32,
    ) -> Result<Generated, SlotError> {
        let branch_row: Option<(Uuid, String, Option<serde_json::Value>)> = sqlx::query_as(
            "SELECT organization_id, timezone, opening_hours FROM branches WHERE id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let Some((org_id, timezone, opening_hours)) = branch_row else {
            return Err(SlotError::NotFound);
        };

        let rule_row: Option<(i32, i32, i32)> = sqlx::query_as(
            "SELECT slot_minutes, capacity, horizon_days FROM booking_slot_rules WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let Some((slot_minutes, capacity, horizon_days)) = rule_row else {
            return Ok(Generated {
                added: 0,
                closed: 0,
            });
        };

        let rule = SlotRule {
            slot_minutes,
            capacity,
            horizon_days,
        };

        let closure_rows: Vec<(NaiveDate, String)> =
            sqlx::query_as("SELECT closed_on, reason FROM branch_closures WHERE branch_id = $1")
                .bind(branch.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let closures: Vec<BranchClosure> = closure_rows
            .into_iter()
            .map(|(closed_on, reason)| BranchClosure { closed_on, reason })
            .collect();

        let hours =
            BranchHours::new(&timezone, opening_hours.as_ref()).map_err(SlotError::Store)?;

        let planned = SlotGenerator::slots_for(&rule, &hours, &closures, from, days);

        let to_date = from + chrono::Duration::days(days as i64);

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let existing_rows: Vec<ExistingSlotRow> = sqlx::query_as(
            "SELECT id, start_at, end_at, capacity, reserved, status \
             FROM booking_slots \
             WHERE branch_id = $1 AND service_date >= $2 AND service_date < $3 \
             FOR UPDATE",
        )
        .bind(branch.as_str())
        .bind(from)
        .bind(to_date)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        let mut existing_by_start: std::collections::HashMap<DateTime<Utc>, ExistingSlotInfo> =
            existing_rows
                .into_iter()
                .map(|(id, start_at, end_at, capacity, reserved, status)| {
                    (
                        start_at,
                        ExistingSlotInfo {
                            id,
                            end_at,
                            capacity,
                            reserved,
                            status,
                        },
                    )
                })
                .collect();

        let now = Utc::now();
        let mut added = 0;
        for p in &planned {
            if let Some(info) = existing_by_start.get_mut(&p.start_at) {
                if info.reserved == 0 && p.start_at > now {
                    // Ruling 2: an empty future slot (reserved = 0, start_at > now) whose start_at equals a new slot's start
                    // is reshaped in place: its end_at and capacity are updated to the new rule (never status).
                    if info.end_at != p.end_at || info.capacity != rule.capacity {
                        sqlx::query(
                            "UPDATE booking_slots \
                             SET end_at = $2, capacity = $3, updated_at = now() \
                             WHERE id = $1",
                        )
                        .bind(info.id)
                        .bind(p.end_at)
                        .bind(rule.capacity)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| SlotError::Store(AppError::Database(e)))?;

                        info.end_at = p.end_at;
                        info.capacity = rule.capacity;
                    }
                }
                // If reserved > 0 or start_at <= now: keeps its old shape.
            } else {
                let slot_id = Uuid::new_v4();
                let res = sqlx::query(
                    "INSERT INTO booking_slots ( \
                         id, organization_id, branch_id, service_date, start_at, end_at, \
                         capacity, reserved, status, created_at, updated_at \
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'OPEN', now(), now()) \
                     ON CONFLICT (branch_id, start_at) DO NOTHING",
                )
                .bind(slot_id)
                .bind(org_id)
                .bind(branch.as_str())
                .bind(p.service_date)
                .bind(p.start_at)
                .bind(p.end_at)
                .bind(rule.capacity)
                .execute(&mut *tx)
                .await
                .map_err(|e| SlotError::Store(AppError::Database(e)))?;

                if res.rows_affected() > 0 {
                    added += 1;
                    existing_by_start.insert(
                        p.start_at,
                        ExistingSlotInfo {
                            id: slot_id,
                            end_at: p.end_at,
                            capacity: rule.capacity,
                            reserved: 0,
                            status: "OPEN".to_string(),
                        },
                    );
                }
            }
        }

        let planned_set: std::collections::HashSet<(DateTime<Utc>, DateTime<Utc>)> =
            planned.iter().map(|p| (p.start_at, p.end_at)).collect();

        let mut ids_to_close = Vec::new();
        for (start_at, info) in &existing_by_start {
            if info.status == "OPEN"
                && info.reserved == 0
                && *start_at > now
                && !planned_set.contains(&(*start_at, info.end_at))
            {
                ids_to_close.push(info.id);
            }
        }

        let closed = ids_to_close.len() as u32;
        if !ids_to_close.is_empty() {
            sqlx::query(
                "UPDATE booking_slots \
                 SET status = 'CLOSED', updated_at = now() \
                 WHERE id = ANY($1)",
            )
            .bind(&ids_to_close)
            .execute(&mut *tx)
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;
        }

        tx.commit()
            .await
            .map_err(|e| SlotError::Store(AppError::Database(e)))?;

        Ok(Generated { added, closed })
    }
}
