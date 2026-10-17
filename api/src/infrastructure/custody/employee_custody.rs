//! `PgEmployeeCustody`: what the owner's employee screen needs from custody.
//!
//! `POST /api/employees` `disable` asks whether the employee still holds cash or
//! has work pending before closing the account: an OPEN shift of their own, or a
//! PENDING hand-over as sender or receiver (CA-10 #626 Q3).

use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};
use xc_shift::domain::shift::value_objects::balances::Balances;

use crate::application::custody::employee_custody::EmployeeCustody;

/// Answers `EmployeeCustody` against PostgreSQL on the pool or inside the caller's
/// branch transaction.
#[derive(Clone)]
pub struct PgEmployeeCustody {
    pool: PgPool,
}

impl PgEmployeeCustody {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// moves to the core
async fn open_of_teller_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    teller: &str,
) -> AppResult<Option<Shift>> {
    let rows: Vec<(String, String, DateTime<Utc>, String, String, String)> = sqlx::query_as(
        "SELECT id, branch_id, opened_at, opened_by, close_note, open_request_id \
         FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
    )
    .bind(teller)
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    if rows.len() > 1 {
        return Err(AppError::Internal(format!(
            "teller {teller} has {} OPEN shifts",
            rows.len()
        )));
    }

    Ok(rows.into_iter().next().map(
        |(id, branch_id, opened_at, opened_by, note, open_request_id)| Shift {
            id,
            branch_id: BranchId::new(branch_id),
            status: ShiftStatus::Open,
            opened_at,
            opened_by,
            opening: Balances::empty(),
            expected: Balances::empty(),
            closed_at: None,
            closed_by: None,
            counted: None,
            difference: None,
            note,
            open_request_id,
            close_request_id: None,
        },
    ))
}

async fn holds_cash_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    _branch: &BranchId,
    employee_id: &str,
) -> AppResult<bool> {
    let user_id: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM users WHERE legacy_employee_id = $1 OR id::text = $1 LIMIT 1",
    )
    .bind(employee_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    let Some(user_id) = user_id else {
        return Ok(false);
    };

    if open_of_teller_in(tx, &user_id).await?.is_some() {
        return Ok(true);
    }

    let pending_handover: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 \
             FROM handovers h \
             LEFT JOIN shifts s ON s.id = h.from_shift_id \
             LEFT JOIN cash_counts c ON c.id = h.count_id \
             WHERE h.status = 'PENDING' \
               AND (h.receiver_id = $1::uuid OR s.teller_id = $1::uuid OR c.counted_by = $1::uuid) \
         )",
    )
    .bind(&user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    Ok(pending_handover)
}

impl EmployeeCustody for PgEmployeeCustody {
    fn holds_cash_or_pending<'a>(
        &'a self,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + 'a>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
            holds_cash_in(&mut tx, branch, employee_id).await
        })
    }

    fn holds_cash_or_pending_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + Send + 'a>> {
        Box::pin(holds_cash_in(tx, branch, employee_id))
    }
}
