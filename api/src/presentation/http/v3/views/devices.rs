//! The `devices` view (§7.1).

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
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::devices::printer_models::{DrawerSettings, PrinterModels};
use crate::infrastructure::devices::printer_settings::{PrinterKind, PrinterSettings};
use crate::presentation::dto::v3::common::{Money, NoFilter};
use crate::presentation::dto::v3::devices::{DeviceBranch, DevicesOverview};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct DevicesBranchView {
    pool: PgPool,
    repo: PgDeviceRepository,
    printer_models: Arc<PrinterModels>,
}

impl DevicesBranchView {
    fn new(pool: PgPool, printer_models: Arc<PrinterModels>) -> Self {
        let repo = PgDeviceRepository::new(pool.clone());
        Self {
            pool,
            repo,
            printer_models,
        }
    }
}

impl BranchView for DevicesBranchView {
    const NAME: &'static str = "devices";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Devices, "snapshot");
    type Data = DevicesOverview;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let repo = self.repo.clone();
        let printer_models = self.printer_models.clone();
        let branch = branch.clone();
        async move {
            // 1. Branches list
            let branch_rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT id, name FROM branches ORDER BY (id = 'MAIN') DESC, created_at ASC, id ASC",
            )
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let branches = branch_rows
                .into_iter()
                .map(|(id, name)| DeviceBranch { id, name })
                .collect();

            // 2. Settings (printer_online, printer_kind, printer_model, has_drawer)
            let (printer_settings, drawer_settings) = {
                let mut tx = pool.begin().await.map_err(AppError::Database)?;
                let printer = PrinterSettings::read_in(&mut tx, &repo, &branch).await?;
                let drawer =
                    DrawerSettings::read_in(&mut tx, &repo, &branch, &printer_models).await?;
                let _ = tx.rollback().await;
                (printer, drawer)
            };

            let printer_online = printer_settings.online;
            let printer_kind = match printer_settings.kind {
                PrinterKind::Simulated => "SIMULATED".to_string(),
                PrinterKind::Network(..) => "NETWORK".to_string(),
            };
            let printer_model = drawer_settings.model;
            let has_drawer = drawer_settings.has_drawer;

            // 3. Active drawer agent
            let drawer_agent_active: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM device_agents WHERE branch_id = $1 AND revoked_at IS NULL)",
            )
            .bind(branch.as_str())
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            // 4. Balances: THB and USD
            let open_shift: Option<String> = sqlx::query_scalar(
                "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'",
            )
            .bind(branch.as_str())
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            let (balance_thb, balance_usd) = if let Some(shift_id) = open_shift {
                let rows: Vec<(String, String)> = sqlx::query_as(
                    "SELECT code::text, COALESCE(expected::text, '0.00') FROM shift_balances WHERE shift_id = $1",
                )
                .bind(shift_id)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

                let map: HashMap<String, String> =
                    rows.into_iter().map(|(c, v)| (c.trim().to_string(), v)).collect();
                (
                    Money(map.get("THB").cloned().unwrap_or_else(|| "0.00".to_string())),
                    Money(map.get("USD").cloned().unwrap_or_else(|| "0.00".to_string())),
                )
            } else {
                let last_shift: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM shifts WHERE branch_id = $1 ORDER BY seq DESC LIMIT 1",
                )
                .bind(branch.as_str())
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                if let Some(shift_id) = last_shift {
                    let rows: Vec<(String, String)> = sqlx::query_as(
                        "SELECT code::text, COALESCE(counted::text, '0.00') FROM shift_balances WHERE shift_id = $1",
                    )
                    .bind(shift_id)
                    .fetch_all(&pool)
                    .await
                    .map_err(AppError::Database)?;

                    let map: HashMap<String, String> =
                        rows.into_iter().map(|(c, v)| (c.trim().to_string(), v)).collect();
                    (
                        Money(map.get("THB").cloned().unwrap_or_else(|| "0.00".to_string())),
                        Money(map.get("USD").cloned().unwrap_or_else(|| "0.00".to_string())),
                    )
                } else {
                    (Money("0.00".to_string()), Money("0.00".to_string()))
                }
            };

            // 5. Completed count (LIVE only)
            let completed_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM transactions WHERE branch_id = $1 AND mode = 'LIVE' AND status = 'COMPLETED'",
            )
            .bind(branch.as_str())
            .fetch_one(&pool)
            .await
            .map_err(AppError::Database)?;

            Ok(DevicesOverview {
                branch_id: branch.as_str().to_owned(),
                branches,
                printer_online,
                printer_kind,
                printer_model,
                has_drawer,
                drawer_agent_active,
                balance_thb,
                balance_usd,
                completed_count,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let printer_models = Arc::new(
        PrinterModels::new(state.settings.devices.printer_models.clone())
            .unwrap_or_else(|_| PrinterModels::built_in()),
    );
    ViewEndpoint::register(
        cfg,
        Arc::new(DevicesBranchView::new(state.db.clone(), printer_models)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeviceBranch::export_all(&cfg)?;
    DevicesOverview::export_all(&cfg)?;
    Ok(())
}
