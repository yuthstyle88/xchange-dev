//! The `cash_transfers` view (CA-8 #624; design `cash-shift-transfer.md` §4.7): the transfers from or to the branch,
//! newest first, paged by `limit` and `before`, filtered by `status` and `kind` (the table's values, D:21-23). Area row
//! `cashTransfers` (G2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash::{
    CashTransferLineRow, CashTransferRow, CashTransfersFilter,
};
use crate::presentation::http::v3::cash_transfer_rows::{
    TRANSFER_COLUMNS, TransferSqlRow, with_lines,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct CashTransfersView {
    pool: PgPool,
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
            let rows: Vec<TransferSqlRow> = sqlx::query_as(AssertSqlSafe(format!(
                "SELECT {TRANSFER_COLUMNS} \
                 FROM cash_transfers t \
                 WHERE (t.from_branch_id = $1 OR t.to_branch_id = $1) \
                   AND ($2::text IS NULL OR t.status = $2) \
                   AND ($3::text IS NULL OR t.kind = $3) \
                   AND ($4::text IS NULL OR (t.created_at, t.id) < \
                        (SELECT b.created_at, b.id FROM cash_transfers b WHERE b.id::text = $4 AND (b.from_branch_id = $1 OR b.to_branch_id = $1))) \
                 ORDER BY t.created_at DESC, t.id DESC \
                 LIMIT $5"
            )))
            .bind(&branch)
            .bind(filter.status.as_deref())
            .bind(filter.kind.as_deref())
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            with_lines(&pool, rows).await
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
