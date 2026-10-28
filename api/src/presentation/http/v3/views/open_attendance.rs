//! Staff corrections §8 note 4 (SC-A4): the `open_attendance` view. Other employees' clock-ins at the branch that
//! are still open and began on a day before today in the branch timezone: what a manager may close with
//! `attendanceCloseFor`. Newest clock-in first.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::open_attendance::OpenAttendanceRow;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

pub struct OpenAttendanceView {
    pool: PgPool,
}

impl PagedView for OpenAttendanceView {
    const NAME: &'static str = "open_attendance";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "openAttendance");
    type Row = OpenAttendanceRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let caller = who.user_id.clone();
        async move {
            let _ = (&pool, &before);
            todo!("SC-A4: open clock-ins of {branch:?} not {caller}, limit {limit}")
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(OpenAttendanceView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::open_attendance::export_types(dir)
}
