//! RA-2b (#1429): `GET /api/v3/views/my_rate_alerts` reads the caller's own rate alerts.

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
use crate::presentation::dto::v3::rate_alerts::{MyRateAlertItem, MyRateAlertsData};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct MyRateAlertsView {
    pool: PgPool,
}

impl MyRateAlertsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for MyRateAlertsView {
    const NAME: &'static str = "my_rate_alerts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "myRateAlerts");
    type Data = MyRateAlertsData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let customer_id = who.customer_id.clone();
        async move {
            let Some(cid) = customer_id else {
                return Ok(MyRateAlertsData { alerts: vec![] });
            };

            let rows: Vec<(String, i32, String, String, String, Decimal, bool)> = sqlx::query_as(
                "SELECT id::text, version, currency_code, shop_side, condition, target, enabled \
                 FROM rate_alerts \
                 WHERE customer_id = $1 AND deleted_at IS NULL \
                 ORDER BY created_at DESC",
            )
            .bind(cid)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let alerts = rows
                .into_iter()
                .map(|(id, version, currency, shop_side, condition, target, enabled)| {
                    let customer_side = match shop_side.as_str() {
                        "SELL" => "CUSTOMER_BUY",
                        "BUY" => "CUSTOMER_SELL",
                        other => other,
                    };
                    MyRateAlertItem {
                        id,
                        version: i64::from(version),
                        currency,
                        customer_side: customer_side.to_string(),
                        condition,
                        target: target.to_string(),
                        enabled,
                    }
                })
                .collect();

            Ok(MyRateAlertsData { alerts })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(MyRateAlertsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyRateAlertItem::export_all(&cfg)?;
    MyRateAlertsData::export_all(&cfg)?;
    Ok(())
}
