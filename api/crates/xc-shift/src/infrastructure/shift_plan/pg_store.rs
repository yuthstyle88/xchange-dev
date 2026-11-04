//! Postgres `shift_templates` store (configurable-shifts.md §3.1, ticket CS-2).

use chrono::{DateTime, NaiveTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::shift_plan::repository::ShiftTemplateStore;
use crate::domain::shift_plan::shift_plan::ShiftPlan;
use crate::domain::shift_plan::shift_template::{ShiftTemplate, ShiftTemplateInput};

#[derive(Debug, Clone, Copy, Default)]
pub struct PgShiftTemplateStore;

type TemplateRow = (
    String,
    String,
    NaiveTime,
    NaiveTime,
    Option<NaiveTime>,
    Option<NaiveTime>,
    i16,
);

async fn fetch_live(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
) -> AppResult<Vec<ShiftTemplate>> {
    let rows: Vec<TemplateRow> = sqlx::query_as(
        "SELECT id::text, name, starts_at, ends_at, break_starts_at, break_ends_at, position \
         FROM shift_templates \
         WHERE branch_id = $1 AND retired_at IS NULL \
         ORDER BY position, id",
    )
    .bind(branch.as_str())
    .fetch_all(&mut **tx)
    .await?;

    let templates = rows
        .into_iter()
        .map(
            |(id, name, starts_at, ends_at, break_starts_at, break_ends_at, position)| {
                let break_window = match (break_starts_at, break_ends_at) {
                    (Some(s), Some(e)) => Some((s, e)),
                    _ => None,
                };
                ShiftTemplate {
                    id,
                    name,
                    starts_at,
                    ends_at,
                    break_window,
                    position,
                }
            },
        )
        .collect();

    Ok(templates)
}

impl ShiftTemplateStore for PgShiftTemplateStore {
    fn live_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftTemplate>>> {
        Box::pin(async move { fetch_live(tx, branch).await })
    }

    fn replace_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        org: &'a str,
        inputs: &'a [ShiftTemplateInput],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<ShiftTemplate>>> {
        Box::pin(async move {
            let mut kept_uuids = Vec::new();
            for input in inputs {
                if let Some(id_str) = &input.id {
                    let uuid = uuid::Uuid::parse_str(id_str)
                        .map_err(|_| AppError::Validation("shiftTemplateUnknown".to_string()))?;
                    kept_uuids.push(uuid);
                }
            }

            // 1. Retire live rows whose id is not in inputs.
            if kept_uuids.is_empty() {
                sqlx::query(
                    "UPDATE shift_templates \
                     SET retired_at = $1, updated_at = $1 \
                     WHERE branch_id = $2 AND retired_at IS NULL",
                )
                .bind(now)
                .bind(branch.as_str())
                .execute(&mut **tx)
                .await?;
            } else {
                sqlx::query(
                    "UPDATE shift_templates \
                     SET retired_at = $1, updated_at = $1 \
                     WHERE branch_id = $2 AND retired_at IS NULL AND NOT (id = ANY($3))",
                )
                .bind(now)
                .bind(branch.as_str())
                .bind(&kept_uuids)
                .execute(&mut **tx)
                .await?;
            }

            // 2. Avoid unique index collision by moving kept rows' position to position + 1000 and temporary name.
            sqlx::query(
                "UPDATE shift_templates \
                 SET position = position + 1000, name = id::text \
                 WHERE branch_id = $1 AND retired_at IS NULL",
            )
            .bind(branch.as_str())
            .execute(&mut **tx)
            .await?;

            // 3. Update each input with an id.
            for (index, input) in inputs.iter().enumerate() {
                if let Some(id_str) = &input.id {
                    let uuid = uuid::Uuid::parse_str(id_str)
                        .map_err(|_| AppError::Validation("shiftTemplateUnknown".to_string()))?;
                    let (break_start, break_end) = match input.break_window {
                        Some((s, e)) => (Some(s), Some(e)),
                        None => (None, None),
                    };
                    let res = sqlx::query(
                        "UPDATE shift_templates \
                         SET name = $1, starts_at = $2, ends_at = $3, break_starts_at = $4, break_ends_at = $5, \
                             position = $6, updated_at = $7 \
                         WHERE id = $8 AND branch_id = $9 AND retired_at IS NULL",
                    )
                    .bind(&input.name)
                    .bind(input.starts_at)
                    .bind(input.ends_at)
                    .bind(break_start)
                    .bind(break_end)
                    .bind(index as i16)
                    .bind(now)
                    .bind(uuid)
                    .bind(branch.as_str())
                    .execute(&mut **tx)
                    .await?;

                    if res.rows_affected() == 0 {
                        return Err(AppError::Validation("shiftTemplateUnknown".to_string()));
                    }
                }
            }

            // 4. Insert each input without an id.
            for (index, input) in inputs.iter().enumerate() {
                if input.id.is_none() {
                    let (break_start, break_end) = match input.break_window {
                        Some((s, e)) => (Some(s), Some(e)),
                        None => (None, None),
                    };
                    sqlx::query(
                        "INSERT INTO shift_templates ( \
                             organization_id, branch_id, name, starts_at, ends_at, break_starts_at, break_ends_at, position \
                         ) VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8)",
                    )
                    .bind(org)
                    .bind(branch.as_str())
                    .bind(&input.name)
                    .bind(input.starts_at)
                    .bind(input.ends_at)
                    .bind(break_start)
                    .bind(break_end)
                    .bind(index as i16)
                    .execute(&mut **tx)
                    .await?;
                }
            }

            // 5. Return live templates.
            fetch_live(tx, branch).await
        })
    }

    fn seed_defaults_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        org: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let has_live: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM shift_templates WHERE branch_id = $1 AND retired_at IS NULL)",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await?;

            if has_live {
                return Ok(false);
            }

            for (pos, input) in ShiftPlan::defaults().into_iter().enumerate() {
                let (break_start, break_end) = match input.break_window {
                    Some((s, e)) => (Some(s), Some(e)),
                    None => (None, None),
                };
                sqlx::query(
                    "INSERT INTO shift_templates ( \
                         organization_id, branch_id, name, starts_at, ends_at, break_starts_at, break_ends_at, position \
                     ) VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8)",
                )
                .bind(org)
                .bind(branch.as_str())
                .bind(&input.name)
                .bind(input.starts_at)
                .bind(input.ends_at)
                .bind(break_start)
                .bind(break_end)
                .bind(pos as i16)
                .execute(&mut **tx)
                .await?;
            }

            Ok(true)
        })
    }
}
