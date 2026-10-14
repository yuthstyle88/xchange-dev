//! Verification content route (design §4.5, V10; ticket #1396).
//! `GET /api/verification/content/{token}`

use std::sync::Arc;

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use serde_json::json;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_verification::application::access::document_access::{AccessError, DocumentAccess};
use xc_verification::application::access::download_link::DownloadLink;

use crate::bootstrap::AppState;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let doc_store = state.document_store.clone();
    cfg.route(
        "/api/verification/content/{token}",
        web::get().to(move |req: HttpRequest, path: web::Path<String>| {
            let pool = pool.clone();
            let doc_store = doc_store.clone();
            async move {
                let token = path.into_inner();
                handle_content(&req, &token, &pool, doc_store).await
            }
        }),
    );
}

async fn handle_content(
    req: &HttpRequest,
    token: &str,
    pool: &sqlx::PgPool,
    doc_store: Arc<dyn xc_kernel::document_store::DocumentStore>,
) -> HttpResponse {
    let session = req
        .cookie("__Host-xc_session")
        .map(|c| c.value().to_string())
        .or_else(|| {
            req.headers()
                .get(header::COOKIE)
                .and_then(|v| v.to_str().ok())
                .and_then(|cookies| {
                    for c in cookies.split(';') {
                        let (k, v) = c.trim().split_once('=')?;
                        if k == "__Host-xc_session" {
                            return Some(v.to_string());
                        }
                    }
                    None
                })
        })
        .unwrap_or_default();

    let keys = match xc_customer::get_pii_keys() {
        Ok(k) => k,
        Err(_) => {
            return HttpResponse::InternalServerError().json(json!({
                "error": { "code": "serverError", "message": "PII keys not available" }
            }));
        }
    };

    let audit = Arc::new(PgAuditLog::new(pool.clone()));
    let download_link = DownloadLink::new(keys);
    let access = DocumentAccess::new(pool.clone(), doc_store, audit, download_link);

    let now = Utc::now();
    match access.open(token, &session, now).await {
        Ok(content) => HttpResponse::Ok()
            .content_type(content.mime)
            .insert_header((header::CACHE_CONTROL, "private, no-store"))
            .insert_header(("x-content-type-options", "nosniff"))
            .insert_header((header::CONTENT_DISPOSITION, "attachment"))
            .body(content.bytes),
        Err(AccessError::LinkExpired) => HttpResponse::Gone().json(json!({
            "error": { "code": "linkExpired", "message": "the link has expired" }
        })),
        Err(AccessError::LinkRefused) => HttpResponse::Forbidden().json(json!({
            "error": { "code": "linkRefused", "message": "the link is refused" }
        })),
        Err(AccessError::NotFound) => HttpResponse::NotFound().json(json!({
            "error": { "code": "notFound", "message": "document not found" }
        })),
        Err(AccessError::OutOfScope) => HttpResponse::Forbidden().json(json!({
            "error": { "code": "outOfScope", "message": "out of scope" }
        })),
        Err(AccessError::Internal(msg)) => HttpResponse::InternalServerError().json(json!({
            "error": { "code": "serverError", "message": msg }
        })),
    }
}
