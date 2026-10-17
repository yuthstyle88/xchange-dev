//! Postgres implementation of CustodyPages.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::Value;
use sqlx::{PgPool, Row};
use xc_kernel::ids::BranchId;

use super::decode::{
    attendance_from_row, balances_from_json, cutoff_from_json, entity_from_json,
    exception_from_row, hold_from_row, str_to_phase, target_from_json,
};
use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::CashException;
use crate::domain::custody::entities::customer_hold::CustomerHold;
use crate::domain::custody::repositories::custody_pages::{CustodyPages, FloorFacts, OpsRow};
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::shift::value_objects::balances::Balances;

#[derive(Clone)]
pub struct PgCustodyPages {
    pool: PgPool,
}

impl PgCustodyPages {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CustodyPages for PgCustodyPages {
    fn holds<'a>(
        &'a self,
        branch: &'a BranchId,
        before: Option<&'a str>,
        limit: u16,
    ) -> BoxFuture<'a, AppResult<Vec<CustomerHold>>> {
        Box::pin(async move {
            let hold_rows = sqlx::query(
                "SELECT id, data \
                 FROM customer_holds \
                 WHERE branch_id = $1 \
                   AND ($2::text IS NULL OR position < (SELECT position FROM customer_holds WHERE branch_id = $1 AND id = $2)) \
                 ORDER BY position DESC \
                 LIMIT $3",
            )
            .bind(branch.as_str())
            .bind(before)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

            let mut holds = Vec::with_capacity(hold_rows.len());
            for r in &hold_rows {
                holds.push(hold_from_row(r)?);
            }
            Ok(holds)
        })
    }

    fn attendance<'a>(
        &'a self,
        branch: &'a BranchId,
        only: Option<&'a str>,
        before: Option<&'a str>,
        limit: u16,
    ) -> BoxFuture<'a, AppResult<Vec<Attendance>>> {
        Box::pin(async move {
            let att_rows = sqlx::query(
                "SELECT id, employee_id, data \
                 FROM attendance \
                 WHERE branch_id = $1 \
                   AND ($2::text IS NULL OR employee_id = $2) \
                   AND ($3::text IS NULL OR position < (SELECT position FROM attendance WHERE branch_id = $1 AND id = $3)) \
                 ORDER BY position DESC \
                 LIMIT $4",
            )
            .bind(branch.as_str())
            .bind(only)
            .bind(before)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

            let mut attendance = Vec::with_capacity(att_rows.len());
            for r in &att_rows {
                attendance.push(attendance_from_row(r)?);
            }
            Ok(attendance)
        })
    }

    fn floor<'a>(
        &'a self,
        branch: &'a BranchId,
        me: &'a str,
    ) -> BoxFuture<'a, AppResult<FloorFacts>> {
        Box::pin(async move {
            let ops_row = sqlx::query(
                "SELECT phase, entity, target, safe, cutoff, service_opened_at \
                 FROM ops_state WHERE branch_id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&self.pool)
            .await?;

            let ops = match ops_row {
                Some(r) => {
                    let phase_str: String = r.get(0);
                    let phase = str_to_phase("ops_state", "phase", &phase_str)?;
                    let entity: Option<Entity> = match r.get::<Option<Value>, _>(1) {
                        Some(v) if !v.is_null() => {
                            Some(entity_from_json("ops_state", "entity", &v)?)
                        }
                        _ => None,
                    };
                    let target: Option<CashTarget> = match r.get::<Option<Value>, _>(2) {
                        Some(v) if !v.is_null() => {
                            Some(target_from_json("ops_state", "target", &v)?)
                        }
                        _ => None,
                    };
                    let safe: Option<Balances> = match r.get::<Option<Value>, _>(3) {
                        Some(v) if !v.is_null() => {
                            Some(balances_from_json("ops_state", "safe", &v)?)
                        }
                        _ => None,
                    };
                    let cutoff: Option<Cutoff> = match r.get::<Option<Value>, _>(4) {
                        Some(v) if !v.is_null() => {
                            Some(cutoff_from_json("ops_state", "cutoff", &v)?)
                        }
                        _ => None,
                    };
                    let service_opened_at: Option<CustodyTime> = r
                        .get::<Option<DateTime<Utc>>, _>(5)
                        .map(|t| CustodyTime::from_epoch_millis(t.timestamp_millis()));

                    Some(OpsRow {
                        phase,
                        entity,
                        target,
                        safe,
                        cutoff,
                        service_opened_at,
                    })
                }
                None => None,
            };

            let att_row = sqlx::query(
                "SELECT id, employee_id, data \
                 FROM attendance \
                 WHERE branch_id = $1 AND employee_id = $2 \
                 ORDER BY position DESC \
                 LIMIT 1",
            )
            .bind(branch.as_str())
            .bind(me)
            .fetch_optional(&self.pool)
            .await?;

            let my_open_attendance = match att_row {
                Some(r) => {
                    let att = attendance_from_row(&r)?;
                    if att.clock_out.is_none() {
                        Some(att)
                    } else {
                        None
                    }
                }
                None => None,
            };

            Ok(FloorFacts {
                ops,
                my_open_attendance,
            })
        })
    }

    fn exceptions_between<'a>(
        &'a self,
        branch: &'a BranchId,
        from: i64,
        to: i64,
    ) -> BoxFuture<'a, AppResult<Vec<CashException>>> {
        Box::pin(async move {
            let exc_rows = sqlx::query(
                "SELECT id, data \
                 FROM cash_exceptions \
                 WHERE branch_id = $1 \
                   AND (data->>'at')::bigint >= $2 \
                   AND (data->>'at')::bigint < $3 \
                 ORDER BY position ASC",
            )
            .bind(branch.as_str())
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?;

            let mut exceptions = Vec::with_capacity(exc_rows.len());
            for r in &exc_rows {
                exceptions.push(exception_from_row(r)?);
            }

            Ok(exceptions)
        })
    }

    fn open_holds<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<u64>> {
        Box::pin(async move {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM customer_holds WHERE branch_id = $1 AND data->>'status' = 'OPEN'",
            )
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await?;

            Ok(count as u64)
        })
    }
}
