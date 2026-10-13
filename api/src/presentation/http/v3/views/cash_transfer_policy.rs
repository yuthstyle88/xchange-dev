//! CA-8c (#1371): organisation-wide cash transfer policy view (`GET /api/cash-transfer-policy` and
//! `/api/v3/views/cash_transfer_policy`), served via `OrgView`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash_transfer_policy::{
    CashTransferPolicyData, CashTransferPolicyFilter,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::org_view::{OrgFilter, OrgView, OrgViewEndpoint};

pub const ROUTE: &str = "/api/cash-transfer-policy";

pub struct CashTransferPolicyView {
    pool: PgPool,
}

impl CashTransferPolicyView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn scope_ids(scope: &BranchScope) -> Vec<BranchId> {
    match scope {
        BranchScope::Branches(list) => list.clone(),
        BranchScope::All => Vec::new(),
    }
}

impl OrgFilter for CashTransferPolicyFilter {
    fn parse(_query: &BTreeMap<String, String>) -> Result<Self, V3Error> {
        Ok(Self::default())
    }
}

impl OrgView for CashTransferPolicyView {
    const NAME: &'static str = "cash_transfer_policy";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferPolicySave");
    type Filter = CashTransferPolicyFilter;
    type Data = Option<CashTransferPolicyData>;

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
                return Ok(None);
            };

            let row: Option<(String, serde_json::Value, Uuid, DateTime<Utc>)> = sqlx::query_as(
                "SELECT version, limits, set_by, created_at \
                 FROM cash_transfer_policy_versions \
                 WHERE organization_id = $1 \
                 ORDER BY created_at DESC \
                 LIMIT 1",
            )
            .bind(org_id)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            let Some((version, limits_val, set_by, created_at)) = row else {
                return Ok(None);
            };

            let limits: BTreeMap<String, String> = serde_json::from_value(limits_val)
                .unwrap_or_default();

            Ok(Some(CashTransferPolicyData {
                version,
                limits,
                set_by: set_by.to_string(),
                created_at: created_at.to_rfc3339(),
            }))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    OrgViewEndpoint::register(
        cfg,
        ROUTE,
        Arc::new(CashTransferPolicyView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::cash_transfer_policy::export_types(dir)
}
