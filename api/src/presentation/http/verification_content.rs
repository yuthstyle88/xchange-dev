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
use crate::presentation::extractors::actor::caller_session_hash;

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
    let now = Utc::now();
    let Some(session) = caller_session_hash(req, now).await else {
        return HttpResponse::Forbidden().json(json!({
            "error": { "code": "linkRefused", "message": "the link is refused" }
        }));
    };

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

    match access.open(token, &session, now).await {
        Ok(content) => {
            let disposition = content_disposition_for(&content.mime);
            HttpResponse::Ok()
                .content_type(content.mime)
                .insert_header((header::CACHE_CONTROL, "private, no-store"))
                .insert_header(("x-content-type-options", "nosniff"))
                .insert_header((header::CONTENT_DISPOSITION, disposition))
                .body(content.bytes)
        }
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

fn content_disposition_for(mime: &str) -> &'static str {
    let essence = mime.split(';').next().unwrap_or("").trim();
    if essence.eq_ignore_ascii_case("application/pdf")
        || essence.eq_ignore_ascii_case("image/jpeg")
        || essence.eq_ignore_ascii_case("image/png")
    {
        "inline"
    } else {
        "attachment"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_application_pdf_disposition_is_inline() {
        assert_eq!(content_disposition_for("application/pdf"), "inline");
    }

    #[test]
    fn test_image_jpeg_disposition_is_inline() {
        assert_eq!(content_disposition_for("image/jpeg"), "inline");
    }

    #[test]
    fn test_image_png_disposition_is_inline() {
        assert_eq!(content_disposition_for("image/png"), "inline");
    }

    #[test]
    fn test_other_mime_disposition_is_attachment() {
        assert_eq!(content_disposition_for("text/html"), "attachment");
    }

    #[test]
    fn test_mime_with_parameters_disposition_is_inline() {
        assert_eq!(
            content_disposition_for("application/pdf; charset=binary"),
            "inline"
        );
    }
}
