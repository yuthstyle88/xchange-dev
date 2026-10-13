//! SP-1 (#1585, rates design §12.6, §12.10): `GET /api/v3/views/org_starting_prices`
//!
//! Organisation-level view listing historical and current starting prices entered by the owner.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::Utc;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::org_starting_prices::{
    OrgStartingPriceItem, OrgStartingPricesFilter,
};
use crate::presentation::http::v3::org_view::{OrgView, OrgViewEndpoint};

const ROUTE: &str = "/api/org/starting-prices";

fn scope_ids(scope: &BranchScope) -> Vec<BranchId> {
    match scope {
        BranchScope::Branches(list) => list.clone(),
        BranchScope::All => Vec::new(),
    }
}

pub struct OrgStartingPricesView {
    pool: PgPool,
}

impl OrgStartingPricesView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl OrgView for OrgStartingPricesView {
    const NAME: &'static str = "org_starting_prices";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "orgStartingPrices");
    type Filter = OrgStartingPricesFilter;
    type Data = Vec<OrgStartingPriceItem>;

    fn versions_in(
        &self,
        _organization_id: &str,
        scope: &BranchScope,
    ) -> impl Future<Output = AppResult<BTreeMap<BranchId, i64>>> + Send {
        let pool = self.pool.clone();
        let ids: Vec<String> = scope_ids(scope)
            .iter()
            .map(|b| b.as_str().to_owned())
            .collect();
        async move {
            let rows: Vec<(String, i64)> = sqlx::query_as(
                "SELECT branch_id, version FROM branch_versions WHERE branch_id = ANY($1)",
            )
            .bind(ids)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(rows
                .into_iter()
                .map(|(b, v)| (BranchId::new(b), v))
                .collect())
        }
    }

    fn load(
        &self,
        organization_id: &str,
        _scope: &BranchScope,
        _filter: &Self::Filter,
        _changed: &[BranchId],
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let org_id = Uuid::parse_str(organization_id).ok();
        async move {
            let Some(org_id) = org_id else {
                return Ok(Vec::new());
            };

            #[derive(sqlx::FromRow)]
            struct Row {
                currency_code: String,
                version: i32,
                buy: Option<Decimal>,
                sell: Option<Decimal>,
                entered_by: String,
                entered_at: chrono::DateTime<Utc>,
                reason: String,
            }

            let rows: Vec<Row> = sqlx::query_as(
                "SELECT currency_code, version, buy, sell, entered_by, entered_at, reason \
                 FROM org_starting_prices \
                 WHERE organization_id = $1 \
                 ORDER BY currency_code ASC, version DESC",
            )
            .bind(org_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let items = rows
                .into_iter()
                .map(|r| OrgStartingPriceItem {
                    currency: r.currency_code.trim().to_string(),
                    version: r.version,
                    buy: r.buy.map(|d| d.to_string()),
                    sell: r.sell.map(|d| d.to_string()),
                    entered_by: r.entered_by,
                    entered_at: r.entered_at.to_rfc3339(),
                    reason: r.reason,
                })
                .collect();

            Ok(items)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    OrgViewEndpoint::register(
        cfg,
        ROUTE,
        Arc::new(OrgStartingPricesView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::org_starting_prices::export_view_types(dir)
}
