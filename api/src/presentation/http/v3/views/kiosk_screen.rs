//! #6273: the `kiosk_screen` view — a branch's owner-set kiosk screen values for the admin screen.
//!
//! Read from `branch_settings` under `kiosk_screen::SETTING` through `BranchSettingStore`; a branch with no row
//! answers the defaults of `application::kiosk_screen`, every field listed in `defaults`, and `version: null`.

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
use crate::presentation::dto::v3::kiosk_screen::KioskScreenData;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct KioskScreenView {
    pool: PgPool,
}

impl KioskScreenView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for KioskScreenView {
    const NAME: &'static str = "kiosk_screen";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "kioskScreen");
    const VERSIONED: bool = true;
    type Data = KioskScreenData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        async move {
            let _ = pool;
            todo!("#6273: read kiosk_screen::SETTING through BranchSettingStore")
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(KioskScreenView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    KioskScreenData::export_all(&cfg)?;
    Ok(())
}
