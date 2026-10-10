//! Postgres implementation of `TransferRepository`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

use crate::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferError, TransferKind, TransferLine,
    TransferStatus, TransferStep,
};
use crate::domain::transfer::transfer_policy::TransferPolicy;
use crate::domain::transfer::transfer_repository::TransferRepository;

pub struct PgTransferRepository {
    pool: PgPool,
}

impl PgTransferRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct TransferRow {
    id: Uuid,
    kind: String,
    from_branch_id: String,
    to_branch_id: Option<String>,
    from_shift_id: Option<String>,
    to_shift_id: Option<String>,
    status: String,
    created_by: Uuid,
}

#[derive(FromRow)]
struct LineRow {
    currency: String,
    denomination_id: i64,
    face_value: Decimal,
    qty_sent: i32,
    qty_received: Option<i32>,
}

fn map_transfer(row: TransferRow, line_rows: Vec<LineRow>) -> CashTransfer {
    let kind = TransferKind::parse(&row.kind).unwrap_or(TransferKind::Branch);
    let status = TransferStatus::parse(&row.status).unwrap_or(TransferStatus::Draft);

    let lines = line_rows
        .into_iter()
        .map(|lr| TransferLine {
            currency: CurrencyCode::parse(lr.currency.trim()).expect("valid currency"),
            denomination_id: lr.denomination_id,
            face_value: Money::parse(&lr.face_value.to_string()).unwrap_or(Money::ZERO),
            qty_sent: lr.qty_sent,
            qty_received: lr.qty_received,
        })
        .collect();

    CashTransfer {
        id: row.id.to_string(),
        kind,
        from_branch: BranchId::new(row.from_branch_id),
        to_branch: row.to_branch_id.map(BranchId::new),
        from_shift: row.from_shift_id,
        to_shift: row.to_shift_id,
        status,
        created_by: row.created_by.to_string(),
        lines,
    }
}

impl TransferRepository for PgTransferRepository {
    fn find_by_id<'a>(&'a self, id: &'a str) -> BoxFuture<'a, AppResult<Option<CashTransfer>>> {
        Box::pin(async move {
            let row: Option<TransferRow> = sqlx::query_as(
                "SELECT id, kind, from_branch_id, to_branch_id, from_shift_id, to_shift_id, status, created_by \
                 FROM cash_transfers WHERE id = $1::uuid",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

            let row = match row {
                Some(r) => r,
                None => return Ok(None),
            };

            let line_rows: Vec<LineRow> = sqlx::query_as(
                "SELECT l.currency, l.denomination_id, d.face_value, l.qty_sent, l.qty_received \
                 FROM cash_transfer_lines l \
                 JOIN denominations d ON d.id = l.denomination_id \
                 WHERE l.transfer_id = $1::uuid \
                 ORDER BY l.currency, d.face_value",
            )
            .bind(id)
            .fetch_all(&self.pool)
            .await?;

            Ok(Some(map_transfer(row, line_rows)))
        })
    }

    fn find_by_id_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CashTransfer>>> {
        Box::pin(async move {
            let row: Option<TransferRow> = sqlx::query_as(
                "SELECT id, kind, from_branch_id, to_branch_id, from_shift_id, to_shift_id, status, created_by \
                 FROM cash_transfers WHERE id = $1::uuid",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;

            let row = match row {
                Some(r) => r,
                None => return Ok(None),
            };

            let line_rows: Vec<LineRow> = sqlx::query_as(
                "SELECT l.currency, l.denomination_id, d.face_value, l.qty_sent, l.qty_received \
                 FROM cash_transfer_lines l \
                 JOIN denominations d ON d.id = l.denomination_id \
                 WHERE l.transfer_id = $1::uuid \
                 ORDER BY l.currency, d.face_value",
            )
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;

            Ok(Some(map_transfer(row, line_rows)))
        })
    }

    fn insert<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        cmd: &'a NewTransfer,
        who: &'a Actor,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<CashTransfer>> {
        Box::pin(async move {
            let transfer_id = Uuid::new_v4();
            let org_uuid = Uuid::parse_str(&who.organization_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;
            let creator_uuid = Uuid::parse_str(&who.user_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            sqlx::query(
                "INSERT INTO cash_transfers ( \
                    id, organization_id, kind, from_branch_id, to_branch_id, \
                    from_shift_id, to_shift_id, status, created_by, note, \
                    idempotency_key, created_at, updated_at \
                ) \
                VALUES ($1, $2, $3, $4, $5, $6, $7, 'DRAFT', $8, $9, $10, $11, $11)",
            )
            .bind(transfer_id)
            .bind(org_uuid)
            .bind(cmd.kind.as_str())
            .bind(cmd.from_branch.as_str())
            .bind(cmd.to_branch.as_ref().map(|b| b.as_str()))
            .bind(cmd.from_shift.as_deref())
            .bind(cmd.to_shift.as_deref())
            .bind(creator_uuid)
            .bind(cmd.note.as_deref())
            .bind(&cmd.idempotency_key)
            .bind(now)
            .execute(&mut **tx)
            .await?;

            for line in &cmd.lines {
                sqlx::query(
                    "INSERT INTO cash_transfer_lines ( \
                        transfer_id, organization_id, currency, denomination_id, \
                        qty_sent, qty_received, created_at \
                    ) \
                    VALUES ($1, $2, $3, $4, $5, NULL, $6)",
                )
                .bind(transfer_id)
                .bind(org_uuid)
                .bind(line.currency.as_str())
                .bind(line.denomination_id)
                .bind(line.qty)
                .bind(now)
                .execute(&mut **tx)
                .await?;
            }

            let loaded = self
                .find_by_id_in(tx, &transfer_id.to_string())
                .await?
                .expect("inserted transfer exists");

            Ok(loaded)
        })
    }

    fn set_approval_request_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        approval_request_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let uuid =
                Uuid::parse_str(id).map_err(|e| pos_error::AppError::Validation(e.to_string()))?;
            let req_uuid = Uuid::parse_str(approval_request_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            sqlx::query(
                "UPDATE cash_transfers \
                 SET approval_request_id = $1, updated_at = $2 \
                 WHERE id = $3",
            )
            .bind(req_uuid)
            .bind(now)
            .bind(uuid)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn update_status<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        status: TransferStatus,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let uuid =
                Uuid::parse_str(id).map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            sqlx::query(
                "UPDATE cash_transfers \
                 SET status = $1, updated_at = $2 \
                 WHERE id = $3",
            )
            .bind(status.as_str())
            .bind(now)
            .bind(uuid)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn mark_dispatched<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        by: &'a Actor,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let uuid =
                Uuid::parse_str(id).map_err(|e| pos_error::AppError::Validation(e.to_string()))?;
            let by_uuid = Uuid::parse_str(&by.user_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            let moved = sqlx::query(
                "UPDATE cash_transfers \
                 SET status = 'DISPATCHED', dispatched_at = $1, dispatched_by = $2, updated_at = $1 \
                 WHERE id = $3 AND status = 'APPROVED'",
            )
            .bind(now)
            .bind(by_uuid)
            .bind(uuid)
            .execute(&mut **tx)
            .await?
            .rows_affected();
            refuse_if_already_moved(tx, uuid, moved, TransferStep::Dispatch).await
        })
    }

    fn mark_received<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        by: &'a Actor,
        lines: &'a [NewTransferLine],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let uuid =
                Uuid::parse_str(id).map_err(|e| pos_error::AppError::Validation(e.to_string()))?;
            let by_uuid = Uuid::parse_str(&by.user_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            // The state change first: a transfer another receipt already moved is refused before
            // any line is touched.
            let moved = sqlx::query(
                "UPDATE cash_transfers \
                 SET status = 'RECEIVED', received_at = $1, received_by = $2, updated_at = $1 \
                 WHERE id = $3 AND status = 'DISPATCHED'",
            )
            .bind(now)
            .bind(by_uuid)
            .bind(uuid)
            .execute(&mut **tx)
            .await?
            .rows_affected();
            refuse_if_already_moved(tx, uuid, moved, TransferStep::Receive).await?;

            for line in lines {
                sqlx::query(
                    "UPDATE cash_transfer_lines \
                     SET qty_received = $1 \
                     WHERE transfer_id = $2 AND currency = $3 AND denomination_id = $4",
                )
                .bind(line.qty)
                .bind(uuid)
                .bind(line.currency.as_str())
                .bind(line.denomination_id)
                .execute(&mut **tx)
                .await?;
            }

            // Set any unmentioned lines to qty_received = 0
            sqlx::query(
                "UPDATE cash_transfer_lines \
                 SET qty_received = 0 \
                 WHERE transfer_id = $1 AND qty_received IS NULL",
            )
            .bind(uuid)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn mark_reconciled<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
        by: &'a Actor,
        note: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let uuid =
                Uuid::parse_str(id).map_err(|e| pos_error::AppError::Validation(e.to_string()))?;
            let by_uuid = Uuid::parse_str(&by.user_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            let moved = sqlx::query(
                "UPDATE cash_transfers \
                 SET status = 'RECONCILED', reconciled_at = $1, reconciled_by = $2, reconcile_note = $3, updated_at = $1 \
                 WHERE id = $4 AND status = 'RECEIVED'",
            )
            .bind(now)
            .bind(by_uuid)
            .bind(note)
            .bind(uuid)
            .execute(&mut **tx)
            .await?
            .rows_affected();
            refuse_if_already_moved(tx, uuid, moved, TransferStep::Reconcile).await
        })
    }

    fn find_latest_policy<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<TransferPolicy>>> {
        Box::pin(async move {
            let org_uuid = Uuid::parse_str(org_id)
                .map_err(|e| pos_error::AppError::Validation(e.to_string()))?;

            let row: Option<(String, Value)> = sqlx::query_as(
                "SELECT version, limits \
                 FROM cash_transfer_policy_versions \
                 WHERE organization_id = $1 \
                 ORDER BY created_at DESC \
                 LIMIT 1",
            )
            .bind(org_uuid)
            .fetch_optional(&mut **tx)
            .await?;

            let (version, limits_val) = match row {
                Some(r) => r,
                None => return Ok(None),
            };

            let mut limits: HashMap<CurrencyCode, Money> = HashMap::new();
            if let Value::Object(map) = limits_val {
                for (cur_str, val) in map {
                    if let Ok(code) = CurrencyCode::parse(cur_str.trim()) {
                        let amount_str = match val {
                            Value::String(s) => s,
                            Value::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        if let Ok(m) = Money::parse(&amount_str) {
                            limits.insert(code, m);
                        }
                    }
                }
            }

            Ok(Some(TransferPolicy::new(version, limits)))
        })
    }
}

/// A conditional state UPDATE that changed no row met a transfer another step already moved: refuse
/// with the state machine's own error, from the state it is in now, so the caller's transaction
/// rolls back and writes nothing.
async fn refuse_if_already_moved(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    rows: u64,
    step: TransferStep,
) -> AppResult<()> {
    if rows == 1 {
        return Ok(());
    }
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM cash_transfers WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some(from) = status.as_deref().and_then(TransferStatus::parse) else {
        return Err(pos_error::AppError::NotFound {
            entity: "CashTransfer",
            id: id.to_string(),
        });
    };
    Err(pos_error::AppError::Domain(
        TransferError::IllegalTransition { from, step }.to_string(),
    ))
}
