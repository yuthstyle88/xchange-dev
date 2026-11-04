//! The `branch_currencies` view (`docs/design/board-currencies.md` §4). SKELETON: `load` is BC-10's.
//! It inherits `cash_counts`'s action and row, as `denominations` does: whoever may count may read it.

use std::collections::HashSet;
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
use xc_rate::domain::currency::branch_currencies::{BranchCurrencies, Display};
use xc_rate::infrastructure::currency::branch_currencies::PgBranchCurrencies;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::branch_currencies::{BranchCurrenciesData, BranchCurrencyRow};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct BranchCurrenciesView {
    pool: PgPool,
}

impl BranchView for BranchCurrenciesView {
    const NAME: &'static str = "branch_currencies";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashCounts");
    type Data = BranchCurrenciesData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move {
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            let branch_rows = PgBranchCurrencies
                .rows_in(&mut tx, &branch)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let held_codes: Vec<String> = sqlx::query_scalar(
                "SELECT trim(currency) FROM inventory_positions \
                 WHERE branch_id = $1 AND quantity <> 0 \
                 ORDER BY currency",
            )
            .bind(branch.as_str())
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::Database)?;
            tx.commit().await.map_err(AppError::Database)?;

            let held_set: HashSet<&str> = held_codes.iter().map(String::as_str).collect();
            let mut rows: Vec<BranchCurrencyRow> = branch_rows
                .into_iter()
                .map(|r| BranchCurrencyRow {
                    code: r.code.as_str().to_owned(),
                    group: r.group.as_str().to_owned(),
                    display: r.display.as_str().to_owned(),
                    sort_order: r.sort_order,
                    held: held_set.contains(r.code.as_str()),
                })
                .collect();

            for code in &held_codes {
                if !rows.iter().any(|r| &r.code == code) {
                    rows.push(BranchCurrencyRow {
                        code: code.clone(),
                        group: "ALL".to_owned(),
                        display: Display::Hide.as_str().to_owned(),
                        sort_order: 1000,
                        held: true,
                    });
                }
            }

            if !rows.iter().any(|r| r.code == "THB") {
                rows.push(BranchCurrencyRow {
                    code: "THB".to_owned(),
                    group: "ALL".to_owned(),
                    display: Display::Hide.as_str().to_owned(),
                    sort_order: 1000,
                    held: false,
                });
            }

            Ok(BranchCurrenciesData { rows })
        }
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
