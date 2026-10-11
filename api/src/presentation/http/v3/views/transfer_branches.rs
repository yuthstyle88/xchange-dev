//! The `transfer_branches` view (CA-8 #624; lane controller's rulings issuecomment-6107955194 and A on
//! issuecomment-6107959194): every branch of the session branch's organisation except the session branch, ordered by
//! name and branch id. A branch has no active state yet, so there is no active filter. Authorised like
//! `transferCreate` (`CashManage`, the same row).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::transfer_branches::{TransferBranchRow, TransferBranches};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct TransferBranchesView {
    pool: PgPool,
}

impl BranchView for TransferBranchesView {
    const NAME: &'static str = "transfer_branches";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferCreate");
    type Data = TransferBranches;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        async move {
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT b.id, b.name FROM branches b \
                 WHERE b.organization_id = (SELECT s.organization_id FROM branches s WHERE s.id = $1) \
                   AND b.id <> $1 \
                 ORDER BY b.name, b.id",
            )
            .bind(&branch)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(TransferBranches {
                rows: rows
                    .into_iter()
                    .map(|(branch_id, name)| TransferBranchRow { branch_id, name })
                    .collect(),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(TransferBranchesView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransferBranchRow::export_all(&cfg)?;
    TransferBranches::export_all(&cfg)?;
    Ok(())
}
