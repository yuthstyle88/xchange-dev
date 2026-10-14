//! The `verification_case` view (§4.5; ticket #1396).
//! `GET /api/v3/views/verification_case?id={case_id}`

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, ViewFilter, authenticate_and_authorize, parse_branch, require_branch,
};

const VIEW_NAME: &str = "verification_case";

pub struct VerificationCaseView {
    pool: PgPool,
}

impl VerificationCaseView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type CaseRow = (
    Uuid,
    Uuid,
    String,
    String,
    String,
    i32,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    Option<String>,
    i32,
);

type AuditRow = (
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
    DateTime<Utc>,
);

type DocumentRow = (
    Uuid,
    String,
    String,
    i32,
    DateTime<Utc>,
    Uuid,
    Option<DateTime<Utc>>,
    String,
);

#[allow(clippy::type_complexity)]
impl BranchView for VerificationCaseView {
    const NAME: &'static str = VIEW_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, VIEW_NAME);
    const VERSIONED: bool = false;
    type Filter = VerificationCaseFilter;
    type Data = VerificationCaseViewAnswer;

    #[allow(clippy::type_complexity)]
    async fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let raw_id = match filter.id.as_deref() {
            Some(id) if !id.trim().is_empty() => id,
            _ => {
                return Err(AppError::Validation("invalid case id".to_string()));
            }
        };
        let case_id = Uuid::parse_str(raw_id)
            .map_err(|_| AppError::Validation("invalid case id".to_string()))?;

        let case_row: Option<CaseRow> = sqlx::query_as(
            "SELECT id, organization_id, customer_id, provider, status, policy_version, branch_id, \
                    submitted_at, decided_at, expires_at, reviewed_by, decision_reason_code, version \
             FROM verification_cases \
             WHERE id = $1",
        )
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let (
            id,
            organization_id,
            customer_id,
            provider,
            status,
            policy_version,
            branch_id,
            submitted_at,
            decided_at,
            expires_at,
            reviewed_by,
            decision_reason_code,
            version,
        ) = match case_row {
            Some(row) => row,
            None => {
                return Err(AppError::NotFound {
                    entity: "verification_case",
                    id: case_id.to_string(),
                });
            }
        };

        if organization_id.to_string() != who.organization_id {
            return Err(AppError::Forbidden);
        }

        let now = Utc::now();
        let scope = BranchScope::of(who, Permission::VerificationReview, now);
        let in_scope = match &branch_id {
            None => matches!(scope, BranchScope::All),
            Some(b) => scope.contains(&BranchId::new(b)),
        };
        if !in_scope {
            return Err(AppError::Forbidden);
        }

        let case_detail = VerificationCaseDetail {
            id: id.to_string(),
            organization_id: organization_id.to_string(),
            customer_id,
            provider,
            status,
            policy_version,
            branch_id,
            submitted_at: submitted_at.map(|t| t.to_rfc3339()),
            decided_at: decided_at.map(|t| t.to_rfc3339()),
            expires_at: expires_at.map(|t| t.to_rfc3339()),
            reviewed_by: reviewed_by.map(|u| u.to_string()),
            decision_reason_code,
            version,
        };

        let audit_rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT id, actor_id, actor_role, scope, branch_id, action, object_ref, \
                    before_redacted, after_redacted, at \
             FROM audit_events \
             WHERE object_ref = $1 \
             ORDER BY at ASC, id ASC",
        )
        .bind(case_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let audit = audit_rows
            .into_iter()
            .map(
                |(
                    id,
                    actor_id,
                    actor_role,
                    scope,
                    branch_id,
                    action,
                    object_ref,
                    before,
                    after,
                    at,
                )| {
                    VerificationCaseAuditRow {
                        id,
                        actor_id,
                        actor_role,
                        scope,
                        branch_id,
                        action,
                        object_ref,
                        before,
                        after,
                        at: at.to_rfc3339(),
                    }
                },
            )
            .collect();

        let doc_rows: Vec<DocumentRow> = sqlx::query_as(
            "SELECT id, document_type, mime, size_bytes, uploaded_at, uploaded_by, \
                    retention_until, access_class \
             FROM verification_documents \
             WHERE case_id = $1 AND removed_at IS NULL AND purged_at IS NULL \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(case_id)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let documents = doc_rows
            .into_iter()
            .map(
                |(
                    id,
                    document_type,
                    mime,
                    size_bytes,
                    uploaded_at,
                    uploaded_by,
                    retention_until,
                    access_class,
                )| {
                    VerificationCaseDocumentRow {
                        id: id.to_string(),
                        document_type,
                        mime,
                        size_bytes,
                        uploaded_at: uploaded_at.to_rfc3339(),
                        uploaded_by: uploaded_by.to_string(),
                        retention_until: retention_until.map(|t| t.to_rfc3339()),
                        access_class,
                    }
                },
            )
            .collect();

        Ok(VerificationCaseViewAnswer {
            case_detail,
            audit,
            documents,
        })
    }

    fn refusal(&self, err: &AppError) -> Option<V3Error> {
        match err {
            AppError::NotFound { .. } => Some(V3Error::new(
                "notFound",
                None,
                "case not found",
                StatusCode::NOT_FOUND,
            )),
            AppError::Forbidden => Some(V3Error::new(
                "outOfScope",
                None,
                "branch out of scope",
                StatusCode::FORBIDDEN,
            )),
            AppError::Validation(msg) => Some(V3Error::new(
                "invalidRequest",
                Some("id"),
                msg,
                StatusCode::BAD_REQUEST,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case.ts")]
pub struct VerificationCaseFilter {
    pub id: Option<String>,
}

impl ViewFilter for VerificationCaseFilter {
    fn check(&self) -> Result<(), V3Error> {
        match self.id.as_deref() {
            Some(id) if Uuid::parse_str(id).is_ok() => Ok(()),
            _ => Err(V3Error::new(
                "invalidRequest",
                Some("id"),
                "invalid case id",
                StatusCode::BAD_REQUEST,
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case.ts")]
pub struct VerificationCaseDetail {
    pub id: String,
    pub organization_id: String,
    pub customer_id: String,
    pub provider: String,
    pub status: String,
    pub policy_version: i32,
    pub branch_id: Option<String>,
    pub submitted_at: Option<String>,
    pub decided_at: Option<String>,
    pub expires_at: Option<String>,
    pub reviewed_by: Option<String>,
    pub decision_reason_code: Option<String>,
    pub version: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case.ts")]
pub struct VerificationCaseAuditRow {
    #[ts(type = "number")]
    pub id: i64,
    pub actor_id: String,
    pub actor_role: String,
    pub scope: String,
    pub branch_id: Option<String>,
    pub action: String,
    pub object_ref: String,
    #[ts(type = "any | null")]
    pub before: Option<serde_json::Value>,
    #[ts(type = "any | null")]
    pub after: Option<serde_json::Value>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case.ts")]
pub struct VerificationCaseDocumentRow {
    pub id: String,
    pub document_type: String,
    pub mime: String,
    pub size_bytes: i32,
    pub uploaded_at: String,
    pub uploaded_by: String,
    pub retention_until: Option<String>,
    pub access_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "verification_case.ts")]
pub struct VerificationCaseViewAnswer {
    #[serde(rename = "case")]
    pub case_detail: VerificationCaseDetail,
    pub audit: Vec<VerificationCaseAuditRow>,
    pub documents: Vec<VerificationCaseDocumentRow>,
}

async fn handle(req: HttpRequest, view: web::Data<Arc<VerificationCaseView>>) -> HttpResponse {
    let named = web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    )
    .ok()
    .and_then(|q| q.into_inner().remove("branch"));
    match parse_branch(named.as_deref()) {
        Ok(Some(branch)) => {
            if let Err(e) = require_branch(&req, &branch).await {
                return e.error_response();
            }
        }
        Ok(None) => {}
        Err(()) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    }
    let who = match authenticate_and_authorize(&req, VerificationCaseView::INHERITS).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };

    let filter = match web::Query::<VerificationCaseFilter>::from_query(req.query_string()) {
        Ok(f) => f.into_inner(),
        Err(_) => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };
    if let Err(e) = filter.check() {
        return e.error_response();
    }

    let branch = who
        .branch
        .clone()
        .unwrap_or_else(|| BranchId::new("XC-B01"));

    match view.load(&branch, &who, &filter).await {
        Ok(data) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(data),
        Err(err) => match view.refusal(&err) {
            Some(refusal) => refusal.error_response(),
            None => V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response(),
        },
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(VerificationCaseView::new(state.db.clone()));
    cfg.service(
        web::resource("/api/v3/views/verification_case")
            .app_data(web::Data::new(view))
            .route(web::get().to(handle)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    VerificationCaseFilter::export_all(&cfg)?;
    VerificationCaseDetail::export_all(&cfg)?;
    VerificationCaseAuditRow::export_all(&cfg)?;
    VerificationCaseDocumentRow::export_all(&cfg)?;
    VerificationCaseViewAnswer::export_all(&cfg)?;
    Ok(())
}
