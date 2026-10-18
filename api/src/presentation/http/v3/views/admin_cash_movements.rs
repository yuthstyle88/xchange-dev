//! The `admin_cash_movements` view (§7, §12.6).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::SecondsFormat;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::infrastructure::transfer_moves;
use xc_kernel::ids::BranchId;
use xc_kernel::money::fixed2;
use xc_shift::domain::shift::value_objects::stock_delta::Direction;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_cash_movements::AdminCashMovementRow;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct AdminCashMovementsPagedView {
    pool: PgPool,
}

impl AdminCashMovementsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for AdminCashMovementsPagedView {
    const NAME: &'static str = "admin_cash_movements";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = AdminCashMovementRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();

        async move {
            let rows = transfer_moves::page(&pool, &branch, before.as_deref(), limit).await?;

            Ok(rows
                .into_iter()
                .map(|r| AdminCashMovementRow {
                    id: r.id,
                    created: r.at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    shift_id: r.shift_id,
                    direction: match r.direction {
                        Direction::In => "IN".to_owned(),
                        Direction::Out => "OUT".to_owned(),
                    },
                    currency: r.currency.as_str().to_owned(),
                    amount: fixed2(r.amount.minor_i128()),
                    reason: r.reason,
                    staff: r.by,
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(AdminCashMovementsPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminCashMovementRow::export_all(&cfg)?;
    Ok(())
}
