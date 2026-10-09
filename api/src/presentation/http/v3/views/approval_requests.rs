//! Approval requests view (§4.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::approval_requests::{
    ApprovalRequestItemDto, ApprovalRequestsData, ApprovalRequestsFilter,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct ApprovalRequestsBranchView {
    pool: PgPool,
}

impl ApprovalRequestsBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for ApprovalRequestsBranchView {
    const NAME: &'static str = "approval_requests";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "approvalRead");
    const VERSIONED: bool = false;
    type Data = ApprovalRequestsData;
    type Filter = ApprovalRequestsFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        let status_filter = filter.status.clone();
        let type_filter = filter.r#type.clone();

        async move {
            let mut sql = "SELECT id, type, branch_id, object_id, object_version_hash, \
                           policy_version, proposed_payload, requested_by, requested_at, status \
                           FROM approval_requests WHERE (branch_id = $1 OR branch_id IS NULL)"
                .to_string();

            if status_filter.is_some() {
                sql.push_str(" AND status = $2");
            }
            if type_filter.is_some() {
                if status_filter.is_some() {
                    sql.push_str(" AND type = $3");
                } else {
                    sql.push_str(" AND type = $2");
                }
            }
            sql.push_str(" ORDER BY requested_at DESC");

            let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&branch_id);
            if let Some(ref st) = status_filter {
                query = query.bind(st);
            }
            if let Some(ref tp) = type_filter {
                query = query.bind(tp);
            }

            let rows = query
                .fetch_all(&pool)
                .await
                .map_err(pos_error::AppError::Database)?;

            let mut requests = Vec::new();
            for r in rows {
                let id: Uuid = r.get(0);
                let type_name: String = r.get(1);
                let b_id: Option<String> = r.get(2);
                let obj_id: String = r.get(3);
                let obj_hash: String = r.get(4);
                let pol_ver: String = r.get(5);
                let payload: Value = r.get(6);
                let req_by: String = r.get(7);
                let req_at: DateTime<Utc> = r.get(8);
                let status: String = r.get(9);

                requests.push(ApprovalRequestItemDto {
                    id: id.to_string(),
                    r#type: type_name,
                    branch_id: b_id,
                    object_id: obj_id,
                    object_version_hash: obj_hash,
                    policy_version: pol_ver,
                    proposed_payload: payload,
                    requested_by: req_by,
                    requested_at: req_at.to_rfc3339(),
                    status,
                });
            }

            Ok(ApprovalRequestsData { requests })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(ApprovalRequestsBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ApprovalRequestsFilter::export_all(&cfg)?;
    ApprovalRequestItemDto::export_all(&cfg)?;
    ApprovalRequestsData::export_all(&cfg)?;
    Ok(())
}
