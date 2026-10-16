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
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{ApprovalRequest, RequestId};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::approval_requests::{
    ApprovalDecisionDto, ApprovalRequestItemDto, ApprovalRequestsData, ApprovalRequestsFilter,
};
use crate::presentation::http::v3::commands::make_approval_registry;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct ApprovalRequestsBranchView {
    pool: PgPool,
    registry: Arc<ApprovalRegistry>,
}

impl ApprovalRequestsBranchView {
    pub fn new(pool: PgPool) -> Self {
        let registry = Arc::new(make_approval_registry(pool.clone()));
        Self { pool, registry }
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
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let registry = self.registry.clone();
        let who = who.clone();
        let branch_id = branch.as_str().to_string();
        let status_filter = filter.status.clone();
        let type_filter = filter.r#type.clone();

        async move {
            let mut sql = "SELECT r.id, r.type, r.branch_id, r.object_id, r.object_version_hash, \
                           r.policy_version, r.proposed_payload, r.requested_by, r.requested_at, r.status, \
                           r.idempotency_key, d.approver_id, d.outcome, d.reason, d.at \
                           FROM approval_requests r \
                           LEFT JOIN approval_decisions d ON d.request_id = r.id \
                           WHERE (r.branch_id = $1 OR r.branch_id IS NULL) \
                           AND r.organization_id = (SELECT b.organization_id FROM branches b WHERE b.id = $1)"
                .to_string();

            if status_filter.is_some() {
                sql.push_str(" AND r.status = $2");
            }
            if type_filter.is_some() {
                if status_filter.is_some() {
                    sql.push_str(" AND r.type = $3");
                } else {
                    sql.push_str(" AND r.type = $2");
                }
            }
            sql.push_str(" ORDER BY r.requested_at DESC");

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
                let idempotency_key: String = r.get(10);
                let decision_approver_id: Option<String> = r.get(11);
                let decision_outcome: Option<String> = r.get(12);
                let decision_reason: Option<String> = r.get(13);
                let decision_at: Option<DateTime<Utc>> = r.get(14);

                let request = ApprovalRequest {
                    id: RequestId::new(id.to_string()),
                    type_name: type_name.clone(),
                    branch_id: b_id.clone(),
                    object_id: obj_id.clone(),
                    object_version_hash: obj_hash.clone(),
                    policy_version: pol_ver.clone(),
                    proposed_payload: payload.clone(),
                    requested_by: req_by.clone(),
                    requested_at: req_at,
                    status: status.clone(),
                    idempotency_key,
                };

                let allowed = registry
                    .get(&type_name)
                    .is_some_and(|(_subject, policy)| policy.may_decide(&who, &request).is_ok());

                if allowed {
                    let decision = match (
                        decision_approver_id,
                        decision_outcome,
                        decision_reason,
                        decision_at,
                    ) {
                        (Some(approver_id), Some(outcome), Some(reason), Some(at)) => {
                            Some(ApprovalDecisionDto {
                                approver_id,
                                outcome,
                                reason,
                                at: at.to_rfc3339(),
                            })
                        }
                        _ => None,
                    };

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
                        decision,
                    });
                }
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
    ApprovalDecisionDto::export_all(&cfg)?;
    ApprovalRequestItemDto::export_all(&cfg)?;
    ApprovalRequestsData::export_all(&cfg)?;
    Ok(())
}

