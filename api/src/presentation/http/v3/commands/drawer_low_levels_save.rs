//! `PUT /api/cash-limits/drawer-low-levels` saves a branch's per-currency drawer low-cash levels
//! (`docs/design/blueprint/ledger-inventory.md` §5.1.3 item 2).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use chrono::Utc;
use uuid::Uuid;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::infrastructure::drawer_low_levels::DrawerLowLevels;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, CurrencyCode, Sign};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash_limits::{CashLimitsData, DrawerLowLevelsSaveInput};
use crate::presentation::http::v3;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::org_view::validation_failed;

pub struct DrawerLowLevelsSaveCommand;

impl CommandAction for DrawerLowLevelsSaveCommand {
    const NAME: &'static str = "drawerLowLevelsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashLimitsSave");
    type Input = DrawerLowLevelsSaveInput;
    type Output = CashLimitsData;
    const ROUTE: &'static str = "/api/cash-limits/drawer-low-levels";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let mut seen = BTreeSet::new();
        let mut parsed = Vec::with_capacity(input.levels.len());
        for item in &input.levels {
            let code =
                CurrencyCode::parse(&item.currency).map_err(|_| validation_failed("levels"))?;
            if !seen.insert(code.as_str()) {
                return Err(validation_failed("levels"));
            }
            v3::input::must_exist(tx, "levels", v3::input::RowRef::Currency, code.as_str()).await?;
            let level = v3::input::decimal(
                "levels",
                &item.low_level,
                Bound::numeric(20, 4, Sign::Positive),
            )?;
            parsed.push((code, level));
        }

        let by = Uuid::parse_str(&who.user_id).map_err(|_| server_error())?;
        DrawerLowLevels::replace_in(tx, branch, &parsed, by, Utc::now())
            .await
            .map_err(|_| server_error())?;

        v3::cash_limits::load_in(tx, branch)
            .await
            .map_err(|_| server_error())
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(DrawerLowLevelsSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DrawerLowLevelsSaveInput::export_all(&cfg)?;
    CashLimitsData::export_all(&cfg)?;
    Ok(())
}
