//! The `my_verification` view (ticket #1399, design §4.4, §9.1 VF-8).
//! `GET /api/v3/views/my_verification`

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
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_verification::application::readiness::Readiness;

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, ViewFilter, authenticate_and_authorize, parse_branch, require_branch,
};

const VIEW_NAME: &str = "my_verification";

pub struct MyVerificationView {
    pool: PgPool,
}

impl MyVerificationView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_verification.ts")]
pub struct MyVerificationFilter {}

impl ViewFilter for MyVerificationFilter {
    fn check(&self) -> Result<(), V3Error> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_verification.ts")]
pub struct MyVerificationDocumentMeta {
    pub id: String,
    pub document_type: String,
    pub mime: String,
    pub size_bytes: i32,
    pub uploaded_at: String,
    pub access_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_verification.ts")]
pub struct MyVerificationCaseDetail {
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
    pub decision_reason_code: Option<String>,
    pub version: i32,
    pub documents: Vec<MyVerificationDocumentMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_verification.ts")]
pub struct MyVerificationViewAnswer {
    #[serde(rename = "case")]
    pub case_detail: Option<MyVerificationCaseDetail>,
    pub readiness: String,
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
    Option<String>,
    i32,
);

type DocRow = (Uuid, String, String, i32, DateTime<Utc>, String);

impl BranchView for MyVerificationView {
    const NAME: &'static str = VIEW_NAME;
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, VIEW_NAME);
    const VERSIONED: bool = false;
    type Filter = MyVerificationFilter;
    type Data = MyVerificationViewAnswer;

    async fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let Some(customer_id) = who.customer_id.as_deref() else {
            return Err(AppError::Forbidden);
        };

        let readiness = match Readiness::of(&self.pool, customer_id).await {
            Ok(r) => r.as_str(),
            Err(e) => {
                tracing::error!(error = %e, "readiness query error");
                return Err(AppError::Internal("failed to query readiness".into()));
            }
        };

        let case_row: Option<CaseRow> = sqlx::query_as(
            "SELECT id, organization_id, customer_id, provider, status, policy_version, branch_id, \
                    submitted_at, decided_at, expires_at, decision_reason_code, version \
             FROM verification_cases \
             WHERE customer_id = $1 \
             ORDER BY created_at DESC, id DESC \
             LIMIT 1",
        )
        .bind(customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let case_detail = if let Some((
            id,
            org_id,
            cust_id,
            provider,
            status,
            policy_version,
            branch_id,
            submitted_at,
            decided_at,
            expires_at,
            decision_reason_code,
            version,
        )) = case_row
        {
            let doc_rows: Vec<DocRow> = sqlx::query_as(
                "SELECT id, document_type, mime, size_bytes, uploaded_at, access_class \
                 FROM verification_documents \
                 WHERE case_id = $1 AND removed_at IS NULL AND purged_at IS NULL \
                 ORDER BY uploaded_at ASC, id ASC",
            )
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let documents = doc_rows
                .into_iter()
                .map(|(d_id, d_type, d_mime, d_size, d_uploaded, d_access)| {
                    MyVerificationDocumentMeta {
                        id: d_id.to_string(),
                        document_type: d_type,
                        mime: d_mime,
                        size_bytes: d_size,
                        uploaded_at: d_uploaded.to_rfc3339(),
                        access_class: d_access,
                    }
                })
                .collect();

            Some(MyVerificationCaseDetail {
                id: id.to_string(),
                organization_id: org_id.to_string(),
                customer_id: cust_id,
                provider,
                status,
                policy_version,
                branch_id,
                submitted_at: submitted_at.map(|t| t.to_rfc3339()),
                decided_at: decided_at.map(|t| t.to_rfc3339()),
                expires_at: expires_at.map(|t| t.to_rfc3339()),
                decision_reason_code,
                version,
                documents,
            })
        } else {
            None
        };

        Ok(MyVerificationViewAnswer {
            case_detail,
            readiness: readiness.to_string(),
        })
    }
}

async fn handle(req: HttpRequest, view: web::Data<Arc<MyVerificationView>>) -> HttpResponse {
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
    let who = match authenticate_and_authorize(&req, MyVerificationView::INHERITS).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };

    let Some(_own_customer_id) = who.customer_id.as_deref() else {
        return V3Error::new(
            "outOfScope",
            None,
            "out of scope",
            StatusCode::FORBIDDEN,
        )
        .error_response();
    };

    let filter = match web::Query::<MyVerificationFilter>::from_query(req.query_string()) {
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
    let view = Arc::new(MyVerificationView::new(state.db.clone()));
    cfg.service(
        web::resource("/api/v3/views/my_verification")
            .app_data(web::Data::new(view))
            .route(web::get().to(handle)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyVerificationFilter::export_all(&cfg)?;
    MyVerificationDocumentMeta::export_all(&cfg)?;
    MyVerificationCaseDetail::export_all(&cfg)?;
    MyVerificationViewAnswer::export_all(&cfg)?;
    Ok(())
}
