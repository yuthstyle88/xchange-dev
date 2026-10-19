//! The `branch_stock` view (design `docs/design/blueprint/stock-reservation.md` D-10, D-11, D-13, D-17; Blueprint
//! §03 B05): per currency of a branch, on hand, reserved, available, the floor and the low flag.
//!
//! Skeleton by the B05 sub-controller (claude/6530d490) on `feat/b05-stock`; SR-4 fills `read`. It inherits
//! `admin_cash`'s policy row (the same `/desk/cash` screen, MANAGER / FINANCE) and is not versioned: a reserve
//! moves `reserved` without bumping the branch's version.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::branch_stock::BranchStockData;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

/// The view's figures at `now`, read on `pool` (S14, S17, S22):
/// - one row per currency that has an on-hand figure (`on_hand_by_currency`'s set: THB from the ledger's DRAWER +
///   SAFE, foreign from `inventory_positions`) or a `branch_stock_floors` row; THB first, then A–Z;
/// - `reserved` from `xc_cash::infrastructure::stock_reservations::active_reserved_in` (THB is never reserved,
///   D-15, so its `reserved` is 0);
/// - `low = available < floor` (D-13), `false` with no floor; `transferPrefill` is `Some { branchId, currency }`
///   exactly when `low` (D-17).
pub async fn read(
    pool: &PgPool,
    organization_id: Uuid,
    branch: &BranchId,
    now: DateTime<Utc>,
) -> AppResult<BranchStockData> {
    let _ = (pool, organization_id, branch, now);
    todo!("SR-4")
}

pub struct BranchStockView {
    pool: PgPool,
}

impl BranchStockView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for BranchStockView {
    const NAME: &'static str = "branch_stock";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = false;
    type Data = BranchStockData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let org = who.organization_id.clone();
        async move {
            let org = Uuid::parse_str(&org)
                .map_err(|_| AppError::Internal(format!("branch_stock: organisation {org:?}")))?;
            read(&pool, org, &branch, Utc::now()).await
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(BranchStockView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::branch_stock::export_types(dir)
}
