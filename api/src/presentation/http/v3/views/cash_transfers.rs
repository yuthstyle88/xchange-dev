//! The `cash_transfers` view (CA-8 #624; design `cash-shift-transfer.md` §4.7): the transfers from or to the branch,
//! newest first, paged by `limit` and `before`, filtered by `status` and `kind` (the table's values, D:21-23). Area row
//! `cashTransfers` (G2).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash::{CashTransferLineRow, CashTransferRow, CashTransfersFilter};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct CashTransfersView {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct TransferSqlRow {
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

impl PagedView for CashTransfersView {
    const NAME: &'static str = "cash_transfers";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashTransfers");
    type Row = CashTransferRow;
    type Filter = CashTransfersFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        let filter = filter.clone();
        async move {
            let rows: Vec<TransferSqlRow> = sqlx::query_as(
                "SELECT t.id::text AS id, t.kind, t.status, t.from_branch_id, t.to_branch_id, t.from_shift_id, \
                        t.to_shift_id, t.created_by::text AS created_by, t.created_at, t.note \
                 FROM cash_transfers t \
                 WHERE (t.from_branch_id = $1 OR t.to_branch_id = $1) \
                   AND ($2::text IS NULL OR t.status = $2) \
                   AND ($3::text IS NULL OR t.kind = $3) \
                   AND ($4::text IS NULL OR (t.created_at, t.id) < \
                        (SELECT b.created_at, b.id FROM cash_transfers b WHERE b.id::text = $4)) \
                 ORDER BY t.created_at DESC, t.id DESC \
                 LIMIT $5",
            )
            .bind(&branch)
            .bind(filter.status.as_deref())
            .bind(filter.kind.as_deref())
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
            let lines: Vec<LineSqlRow> = sqlx::query_as(
                "SELECT transfer_id::text AS transfer_id, currency::text AS currency, denomination_id, qty_sent, \
                        qty_received \
                 FROM cash_transfer_lines WHERE transfer_id::text = ANY($1) ORDER BY currency, denomination_id",
            )
            .bind(&ids)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            let mut by_transfer: BTreeMap<String, Vec<CashTransferLineRow>> = BTreeMap::new();
            for l in lines {
                by_transfer.entry(l.transfer_id).or_default().push(CashTransferLineRow {
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
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(CashTransfersView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashTransfersFilter::export_all(&cfg)?;
    CashTransferLineRow::export_all(&cfg)?;
    CashTransferRow::export_all(&cfg)?;
    Ok(())
}
