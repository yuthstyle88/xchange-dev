//! The `rates_draftable` view (`docs/design/board-currencies.md` D-8). SKELETON: `load` is BC-5's.
//! It inherits the reference route, as `rates_reference` does.

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
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rates_draftable::{DraftableCurrency, RatesDraftableData};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct RatesDraftableView {
    #[allow(dead_code)]
    pool: PgPool,
}

impl BranchView for RatesDraftableView {
    const NAME: &'static str = "rates_draftable";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/reference-rates");
    type Data = RatesDraftableData;
    type Filter = NoFilter;

    #[allow(clippy::manual_async_fn)]
    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        async move { todo!("BC-5") }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(RatesDraftableView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DraftableCurrency::export_all(&cfg)?;
    RatesDraftableData::export_all(&cfg)?;
    Ok(())
}
