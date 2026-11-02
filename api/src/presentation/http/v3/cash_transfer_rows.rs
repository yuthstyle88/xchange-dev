//! Shared `CashTransferRow` SQL row and line builder for `cash_transfers` and `my_shift`
//! (design `cash-shift-transfer.md` §4.7).

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;

use crate::presentation::dto::v3::cash::{CashTransferLineRow, CashTransferRow};

pub(crate) const TRANSFER_COLUMNS: &str = "t.id::text AS id, t.kind, t.status, t.from_branch_id, t.to_branch_id, t.from_shift_id, t.to_shift_id, t.created_by::text AS created_by, t.created_at, t.note";

#[derive(sqlx::FromRow)]
pub(crate) struct TransferSqlRow {
    id: String,
    kind: String,
    status: String,
    from_branch_id: String,
    to_branch_id: Option<String>,
    from_shift_id: Option<String>,
    to_shift_id: Option<String>,
    created_by: String,
    created_at: DateTime<Utc>,
    note: Option<String>,
}

#[derive(sqlx::FromRow)]
struct LineSqlRow {
    transfer_id: String,
    currency: String,
    denomination_id: i64,
    qty_sent: i32,
    qty_received: Option<i32>,
}

pub(crate) async fn with_lines(
    pool: &PgPool,
    rows: Vec<TransferSqlRow>,
) -> AppResult<Vec<CashTransferRow>> {
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let lines: Vec<LineSqlRow> = sqlx::query_as(
        "SELECT transfer_id::text AS transfer_id, currency::text AS currency, denomination_id, qty_sent, \
                qty_received \
         FROM cash_transfer_lines WHERE transfer_id::text = ANY($1) ORDER BY currency, denomination_id",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;
    let mut by_transfer: BTreeMap<String, Vec<CashTransferLineRow>> = BTreeMap::new();
    for l in lines {
        by_transfer
            .entry(l.transfer_id)
            .or_default()
            .push(CashTransferLineRow {
                currency: l.currency.trim().to_owned(),
                denomination_id: l.denomination_id,
                qty_sent: l.qty_sent.to_string(),
                qty_received: l.qty_received.map(|q| q.to_string()),
            });
    }

    Ok(rows
        .into_iter()
        .map(|r| CashTransferRow {
            lines: by_transfer.remove(&r.id).unwrap_or_default(),
            id: r.id,
            kind: r.kind,
            status: r.status,
            from_branch_id: r.from_branch_id,
            to_branch_id: r.to_branch_id,
            from_shift_id: r.from_shift_id,
            to_shift_id: r.to_shift_id,
            created_by: r.created_by,
            created_at: r.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
            note: r.note,
        })
        .collect())
}
