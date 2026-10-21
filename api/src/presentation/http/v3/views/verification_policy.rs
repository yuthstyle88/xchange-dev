//! The `verification_policy` view (§4.5; ticket #1878, VF-13).
//! `GET /api/v3/views/verification_policy`

use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};

const VIEW_NAME: &str = "verification_policy";

pub struct VerificationPolicyView {
    pool: PgPool,
}

impl VerificationPolicyView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for VerificationPolicyView {
    const NAME: &'static str = VIEW_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, VIEW_NAME);
    const VERSIONED: bool = true;
    const ORG_WIDE: bool = true;
    type Filter = VerificationPolicyFilter;
    type Data = VerificationPolicyData;

    async fn own_version(&self, _branch: &BranchId, who: &Actor) -> AppResult<Option<i64>> {
        let Ok(org_id) = Uuid::parse_str(&who.organization_id) else {
            return Ok(Some(-1));
        };
        let version: Option<i32> = sqlx::query_scalar(
            "SELECT version FROM verification_policies \
             WHERE organization_id = $1 \
             ORDER BY version DESC \
             LIMIT 1",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(Some(version.map(i64::from).unwrap_or(-1)))
    }

    async fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let org_id = Uuid::parse_str(&who.organization_id).map_err(|_| AppError::Forbidden)?;

        let row: Option<(i32, Vec<String>, i32, bool)> = sqlx::query_as(
            "SELECT version, accepted_document_types, documents_required, branch_review_allowed \
             FROM verification_policies \
             WHERE organization_id = $1 \
             ORDER BY version DESC \
             LIMIT 1",
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let (policy_version, accepted_document_types, documents_required, branch_review_allowed) =
            match row {
                Some(r) => r,
                None => {
                    return Err(AppError::NotFound {
                        entity: "verification_policy",
                        id: who.organization_id.clone(),
                    });
                }
            };

        Ok(VerificationPolicyData {
            policy_version,
            branch_review_allowed,
            accepted_document_types,
            documents_required,
        })
    }

    fn refusal(&self, err: &AppError) -> Option<V3Error> {
        match err {
            AppError::NotFound { .. } => Some(V3Error::new(
                "noVerificationPolicy",
                None,
                "no verification policy",
                StatusCode::BAD_REQUEST,
            )),
            AppError::Forbidden => Some(V3Error::new(
                "outOfScope",
                None,
                "out of scope",
                StatusCode::FORBIDDEN,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Deserialize, TS)]
#[ts(export_to = "verification_policy.ts")]
pub struct VerificationPolicyFilter {
    #[ts(optional, type = "number")]
    pub since: Option<i64>,
}

impl ViewFilter for VerificationPolicyFilter {}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_policy.ts")]
pub struct VerificationPolicyData {
    pub policy_version: i32,
    pub branch_review_allowed: bool,
    pub accepted_document_types: Vec<String>,
    pub documents_required: i32,
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(VerificationPolicyView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationPolicyFilter::export_all(&cfg)?;
    VerificationPolicyData::export_all(&cfg)?;
    Ok(())
}
