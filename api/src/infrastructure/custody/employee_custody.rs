//! `PgEmployeeCustody`: what the owner's employee screen needs from custody.
//!
//! `POST /api/employees` `disable` asks whether the employee still holds cash or
//! has work pending before closing the account: an OPEN shift of their own, or a
//! PENDING hand-over as sender or receiver (CA-10 #626 Q3).

use std::future::Future;
use std::pin::Pin;

use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::application::custody::employee_custody::EmployeeCustody;

const SEEDED_XC_ORG_ID: &str = "00000000-0000-0000-0000-000000000001";

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

// moves to xc-cash (#2957)
pub async fn holds_cash_in(
    tx: &mut sqlx::PgConnection,
    organization_id: &str,
    employee_id: &str,
) -> AppResult<bool> {
    let holds: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 \
             FROM users u \
             WHERE u.organization_id = $1::uuid \
               AND (u.legacy_employee_id = $2 OR u.id::text = $2) \
               AND ( \
                   EXISTS ( \
                       SELECT 1 FROM shifts s \
                       WHERE s.teller_id = u.id AND s.status = 'OPEN' \
                   ) \
                   OR EXISTS ( \
                       SELECT 1 FROM handovers h \
                       LEFT JOIN shifts s ON s.id = h.from_shift_id \
                       LEFT JOIN cash_counts c ON c.id = h.count_id \
                       WHERE h.organization_id = u.organization_id \
                         AND h.status = 'PENDING' \
                         AND (h.receiver_id = u.id OR s.teller_id = u.id OR c.counted_by = u.id) \
                   ) \
               ) \
         )",
    )
    .bind(organization_id)
    .bind(employee_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    Ok(holds)
}

impl EmployeeCustody for PgEmployeeCustody {
    fn holds_cash_or_pending<'a>(
        &'a self,
        _branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + 'a>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
            holds_cash_in(&mut *tx, SEEDED_XC_ORG_ID, employee_id).await
        })
    }

    fn holds_cash_or_pending_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + Send + 'a>> {
        Box::pin(holds_cash_in(&mut **tx, SEEDED_XC_ORG_ID, employee_id))
    }
}
