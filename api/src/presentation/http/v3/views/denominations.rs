//! The `denominations` view (CA-8 #624; lane controller's ruling issuecomment-6107796305): the face values the count
//! and transfer lines name, read-only: only rows with a face value (the catch-all is not countable; amended ruling
//! issuecomment-6107853376), ordered by `currency, face_value, id`. It inherits `cash_counts`'s action and row
//! (`cashCounts`), so whoever may count may read the face values.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::denominations::{DenominationRow, Denominations};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct DenominationsView {
    pool: PgPool,
}

impl BranchView for DenominationsView {
    const NAME: &'static str = "denominations";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashCounts");
    type Data = Denominations;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        async move {
            let rows: Vec<(i64, String, Decimal, String)> = sqlx::query_as(
                "SELECT id, currency_code::text, face_value, group_code FROM denominations \
                 WHERE face_value IS NOT NULL \
                 ORDER BY currency_code, face_value, id",
            )
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(Denominations {
                rows: rows
                    .into_iter()
                    .map(|(id, currency, face_value, group_code)| DenominationRow {
                        id,
                        currency: currency.trim().to_owned(),
                        face_value: face_value.to_string(),
                        group_code,
                    })
                    .collect(),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(DenominationsView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DenominationRow::export_all(&cfg)?;
    Denominations::export_all(&cfg)?;
    Ok(())
}
