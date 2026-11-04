//! The `branch_currencies` view (`docs/design/board-currencies.md` §4). SKELETON: `load` is BC-10's.
//! It inherits `cash_counts`'s action and row, as `denominations` does: whoever may count may read it.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::branch_currencies::{BranchCurrenciesData, BranchCurrencyRow};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct BranchCurrenciesView {
    #[allow(dead_code)]
    pool: PgPool,
}

impl BranchView for BranchCurrenciesView {
    const NAME: &'static str = "branch_currencies";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashCounts");
    type Data = BranchCurrenciesData;
    type Filter = NoFilter;

    #[allow(clippy::manual_async_fn)]
    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        async move { todo!("BC-10") }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(BranchCurrenciesView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchCurrencyRow::export_all(&cfg)?;
    BranchCurrenciesData::export_all(&cfg)?;
    Ok(())
}
