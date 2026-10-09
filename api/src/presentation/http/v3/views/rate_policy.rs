//! Rate policy view (§4.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rate_policy::{RatePolicyData, RatePolicyVersionDto};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct RatePolicyBranchView {
    pool: PgPool,
}

impl RatePolicyBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for RatePolicyBranchView {
    const NAME: &'static str = "rate_policy";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    const VERSIONED: bool = true;
    type Data = RatePolicyData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();

        async move {
            let row = sqlx::query(
                "SELECT id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                        max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
                        approved_by, reason \
                 FROM rate_policy_versions WHERE branch_id = $1 ORDER BY version DESC LIMIT 1",
            )
            .bind(&branch_id)
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let policy = match row {
                Some(r) => {
                    let id: Uuid = r.get(0);
                    let b_id: String = r.get(1);
                    let version: i32 = r.get(2);
                    let buy_pct: Decimal = r.get(3);
                    let sell_pct: Decimal = r.get(4);
                    let guards: Value = r.get(5);
                    let max_age: Option<i32> = r.get(6);
                    let on_stale: Option<String> = r.get(7);
                    let stale_limit: Option<Decimal> = r.get(8);
                    let valid_from: DateTime<Utc> = r.get(9);
                    let valid_to: Option<DateTime<Utc>> = r.get(10);
                    let approved_by: String = r.get(11);
                    let reason: Option<String> = r.get(12);

                    Some(RatePolicyVersionDto {
                        id: id.to_string(),
                        branch_id: b_id,
                        version,
                        buy_adjust_pct: buy_pct.to_string(),
                        sell_adjust_pct: sell_pct.to_string(),
                        guards,
                        max_reference_age_hours: max_age,
                        on_stale,
                        stale_limit_thb: stale_limit.map(|d| d.to_string()),
                        valid_from: valid_from.to_rfc3339(),
                        valid_to: valid_to.map(|t| t.to_rfc3339()),
                        approved_by,
                        reason,
                    })
                }
                None => None,
            };

            Ok(RatePolicyData { policy })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(RatePolicyBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RatePolicyVersionDto::export_all(&cfg)?;
    RatePolicyData::export_all(&cfg)?;
    Ok(())
}
