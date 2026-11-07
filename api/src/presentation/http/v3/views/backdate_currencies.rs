//! The `backdate_currencies` view (`docs/design/backdated-keying.md` §15.2, F46).
//! It inherits the backdated keying command's action: whoever may key a backdated sale may read its currencies.

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
use crate::presentation::dto::v3::backdate_currencies::{
    BackdateCurrenciesData, BackdateCurrencyRow,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct BackdateCurrenciesView {
    pool: PgPool,
}

impl BranchView for BackdateCurrenciesView {
    const NAME: &'static str = "backdate_currencies";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transactionBackdate");
    type Data = BackdateCurrenciesData;
    type Filter = NoFilter;

    /// Every row of `currencies` except THB, ordered by trimmed code, each as a [`BackdateCurrencyRow`] with `unit_size` normalised (`Decimal::normalize`).
    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        async move {
            let rows: Vec<(String, rust_decimal::Decimal, i16)> = sqlx::query_as(
                "SELECT trim(code), unit_size, minor_unit FROM currencies WHERE trim(code) <> 'THB' ORDER BY trim(code)",
            )
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut currencies = Vec::with_capacity(rows.len());
            for (code, unit_size, minor_unit) in rows {
                let minor_unit = u8::try_from(minor_unit)
                    .map_err(|_| AppError::Internal(format!("invalid minor_unit for {code}")))?;
                currencies.push(BackdateCurrencyRow {
                    code,
                    unit_size: unit_size.normalize().to_string(),
                    minor_unit,
                });
            }
            Ok(BackdateCurrenciesData { currencies })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(BackdateCurrenciesView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BackdateCurrencyRow::export_all(&cfg)?;
    BackdateCurrenciesData::export_all(&cfg)?;
    Ok(())
}
