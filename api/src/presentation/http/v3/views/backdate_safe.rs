//! The `backdate_safe` view (`docs/design/backdated-keying.md` §16.3, F56).
//! It inherits the backdated keying command's action: whoever may key a backdated sale may see the SAFE it posts against.

use std::collections::HashMap;
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
use crate::presentation::dto::v3::backdate_safe::{BackdateSafeData, BackdateSafeLine};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct BackdateSafeView {
    pool: PgPool,
}

impl BranchView for BackdateSafeView {
    const NAME: &'static str = "backdate_safe";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transactionBackdate");
    type Data = BackdateSafeData;
    type Filter = NoFilter;

    /// F56a: `crate::application::compat::snapshot::ledger_safe(&pool, branch)`, its non-zero balances as
    /// [`BackdateSafeLine`]s ordered by code, each written to the currency's `currencies.minor_unit` the way
    /// `staff_floor` writes `safe` (`application/staff_floor/reader.rs:162-169`). No SAFE activity ⇒ empty list.
    #[allow(clippy::manual_async_fn)]
    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move {
            let Some(safe) =
                crate::application::compat::snapshot::ledger_safe(&pool, &branch).await?
            else {
                return Ok(BackdateSafeData {
                    balances: Vec::new(),
                });
            };
            let minor_units: HashMap<String, i16> =
                sqlx::query_as("SELECT trim(code), minor_unit FROM currencies")
                    .fetch_all(&pool)
                    .await
                    .map_err(AppError::Database)?
                    .into_iter()
                    .collect();
            let mut balances: Vec<BackdateSafeLine> = safe
                .non_zero()
                .into_iter()
                .map(|(code, money)| {
                    let currency = code.as_str().to_string();
                    let mut d = money.value().normalize();
                    let scale = minor_units.get(&currency).copied().unwrap_or(2).max(0) as u32;
                    if d.scale() < scale {
                        d.rescale(scale);
                    }
                    BackdateSafeLine {
                        currency,
                        amount: d.to_string(),
                    }
                })
                .collect();
            balances.sort_by(|a, b| a.currency.cmp(&b.currency));
            Ok(BackdateSafeData { balances })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(BackdateSafeView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BackdateSafeLine::export_all(&cfg)?;
    BackdateSafeData::export_all(&cfg)?;
    Ok(())
}
