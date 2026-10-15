//! Multipart upload route for customer verification documents (ticket #1399, design §4.4, §9.1 VF-8).
//! `POST /api/app/verification/cases/{id}/documents`

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::document_store::DocumentStore;
use xc_kernel::pii::IdentityType;
use xc_verification::application::case_service::CaseService;
use xc_verification::domain::case::case_actor::CaseActor;
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::customer_actor::CustomerActor;
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::pg_store::PgIdempotencyStore;
use crate::infrastructure::idempotency::store::{Claim, IdempotencyKey, IdempotencyStore};
use crate::presentation::pages;

pub const ROUTE: &str = "/api/app/verification/cases/{id}/documents";
pub const ACTION_NAME: &str = "verificationDocumentUpload";

const TOO_LARGE: &str = "ไฟล์ใหญ่เกิน 15 MB";

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let doc_store = state.document_store.clone();
    cfg.service(
        web::resource(ROUTE)
            .app_data(web::PayloadConfig::new(25 * 1024 * 1024))
            .route(web::post().to(
                move |req: HttpRequest, payload: web::Payload, path: web::Path<String>| {
                    let pool = pool.clone();
                    let doc_store = doc_store.clone();
                    async move {
                        let case_id_str = path.into_inner();
                        handle_upload(&req, payload, &case_id_str, &pool, doc_store).await
                    }
                },
            )),
    );
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

struct ParsedMultipart {
    document_type: Option<String>,
    file: Option<Vec<u8>>,
}

fn parse_multipart(boundary: &str, body: &[u8]) -> Result<ParsedMultipart, &'static str> {
    let delim = format!("--{boundary}").into_bytes();
    let mut document_type = None;
    let mut file = None;

    let mut remaining = body;
    while let Some(start_idx) = find_subslice(remaining, &delim) {
        remaining = &remaining[start_idx + delim.len()..];
        if remaining.starts_with(b"--") {
            break;
        }
        if remaining.starts_with(b"\r\n") {
            remaining = &remaining[2..];
        } else if remaining.starts_with(b"\n") {
            remaining = &remaining[1..];
        }

        let next_delim = find_subslice(remaining, &delim).unwrap_or(remaining.len());
        let part_bytes = &remaining[..next_delim];
        let part_data = if part_bytes.ends_with(b"\r\n") {
            &part_bytes[..part_bytes.len() - 2]
        } else if part_bytes.ends_with(b"\n") {
            &part_bytes[..part_bytes.len() - 1]
        } else {
            part_bytes
        };

        let (header_bytes, content_bytes) = if let Some(sep) = find_subslice(part_data, b"\r\n\r\n")
        {
            (&part_data[..sep], &part_data[sep + 4..])
        } else if let Some(sep) = find_subslice(part_data, b"\n\n") {
            (&part_data[..sep], &part_data[sep + 2..])
        } else {
            continue;
        };

        let header_str = String::from_utf8_lossy(header_bytes);
        if header_str.contains("name=\"documentType\"") {
            let val = String::from_utf8_lossy(content_bytes).trim().to_string();
            if !val.is_empty() {
                document_type = Some(val);
            }
        } else if header_str.contains("name=\"file\"") {
            file = Some(content_bytes.to_vec());
        }
    }

    Ok(ParsedMultipart {
        document_type,
        file,
    })
}

fn map_case_error(err: CaseError) -> HttpResponse {
    let (status, code, msg) = match err {
        CaseError::NoVerificationPolicy => (
            StatusCode::BAD_REQUEST,
            "noVerificationPolicy",
            "no verification policy for organization".to_string(),
        ),
        CaseError::CaseLocked => (
            StatusCode::BAD_REQUEST,
            "caseLocked",
            "case is locked".to_string(),
        ),
        CaseError::DocumentTypeNotAccepted => (
            StatusCode::BAD_REQUEST,
            "documentTypeNotAccepted",
            "document type not accepted".to_string(),
        ),
        CaseError::DocumentsMissing => (
            StatusCode::BAD_REQUEST,
            "documentsMissing",
            "documents missing".to_string(),
        ),
        CaseError::OutOfScope => (
            StatusCode::FORBIDDEN,
            "outOfScope",
            "out of scope".to_string(),
        ),
        CaseError::IdempotencyConflict => (
            StatusCode::CONFLICT,
            "idempotencyConflict",
            "idempotency conflict".to_string(),
        ),
        CaseError::File(sentence) => (
            StatusCode::BAD_REQUEST,
            "validationFailed",
            sentence.to_string(),
        ),
        CaseError::NotFound => (
            StatusCode::NOT_FOUND,
            "notFound",
            "verification case not found".to_string(),
        ),
        CaseError::Store(e) => {
            tracing::error!("verification upload store error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "serverError",
                "internal server error".to_string(),
            )
        }
        CaseError::Documents(e) => {
            tracing::error!("verification upload document store error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "serverError",
                "internal server error".to_string(),
            )
        }
    };

    HttpResponse::build(status)
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(serde_json::json!({
            "error": {
                "code": code,
                "message": msg,
            }
        }))
}

async fn handle_upload(
    req: &HttpRequest,
    payload: web::Payload,
    case_id_str: &str,
    pool: &PgPool,
    doc_store: Arc<dyn DocumentStore>,
) -> HttpResponse {
    let idempotency_key = match req
        .headers()
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
    {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return HttpResponse::BadRequest()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "validationFailed",
                        "field": "idempotencyKey",
                        "message": "Idempotency-Key is required",
                    }
                }));
        }
    };

    let callers = pages::get_callers(req).await;
    let now = Utc::now();
    let action = Action::new(ACTION_NAME);
    let _grant = match Policy::authorize(callers.user.as_ref(), EntryPath::Rpc, &action, None, now)
    {
        Ok(g) => g,
        Err(e) => {
            let (status, code) = match e {
                PolicyError::SignIn => (StatusCode::UNAUTHORIZED, "signIn"),
                PolicyError::RoleNotAllowedHere => (StatusCode::BAD_REQUEST, "roleNotAllowedHere"),
                PolicyError::OutOfScope => (StatusCode::FORBIDDEN, "outOfScope"),
                _ => (StatusCode::FORBIDDEN, "outOfScope"),
            };
            return HttpResponse::build(status)
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": code,
                        "message": e.to_string(),
                    }
                }));
        }
    };

    let Some(user) = callers.user.as_ref() else {
        return HttpResponse::Unauthorized()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({
                "error": {
                    "code": "signIn",
                    "message": "signIn",
                }
            }));
    };

    let Some(customer_id) = user.customer_id.as_deref() else {
        return HttpResponse::Forbidden()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({
                "error": {
                    "code": "outOfScope",
                    "message": "out of scope",
                }
            }));
    };

    let case_id = match Uuid::parse_str(case_id_str) {
        Ok(u) => u,
        Err(_) => {
            return HttpResponse::BadRequest()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "invalidRequest",
                        "field": "id",
                        "message": "invalid case id",
                    }
                }));
        }
    };

    const MAX_BODY_LIMIT: usize = 25 * 1024 * 1024;
    let bytes = match payload.to_bytes_limited(MAX_BODY_LIMIT).await {
        Ok(Ok(b)) => b.to_vec(),
        _ => {
            return HttpResponse::BadRequest()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "validationFailed",
                        "field": "file",
                        "message": TOO_LARGE,
                    }
                }));
        }
    };

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let boundary = content_type
        .split(';')
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("boundary=")
        })
        .map(|b| b.trim_matches('"'))
        .unwrap_or("");

    let parsed = match parse_multipart(boundary, &bytes) {
        Ok(p) => p,
        Err(msg) => {
            return HttpResponse::BadRequest()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "invalidRequest",
                        "message": msg,
                    }
                }));
        }
    };

    let doc_type_str = parsed.document_type.as_deref().unwrap_or("");
    let doc_type = match doc_type_str {
        "PASSPORT" => IdentityType::Passport,
        "NATIONAL_ID" => IdentityType::NationalId,
        "PHONE" => IdentityType::Phone,
        "EMAIL" => IdentityType::Email,
        _ => {
            return HttpResponse::BadRequest()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "documentTypeNotAccepted",
                        "field": "documentType",
                        "message": "document type not accepted",
                    }
                }));
        }
    };

    let Some(file_bytes) = parsed.file else {
        return HttpResponse::BadRequest()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({
                "error": {
                    "code": "invalidRequest",
                    "field": "file",
                    "message": "missing file part",
                }
            }));
    };

    let file_hash = format!("{:x}", Sha256::digest(&file_bytes));
    let req_hash = format!(
        "{:x}",
        Sha256::digest(
            serde_json::json!({
                "caseId": case_id.to_string(),
                "documentType": doc_type.as_str(),
                "fileSha256": file_hash,
            })
            .to_string()
            .as_bytes()
        )
    );

    let idem_store = PgIdempotencyStore::new(pool.clone(), user.organization_id.clone());
    let idem_key = IdempotencyKey::new(idempotency_key.clone());

    let claim_token = match idem_store.claim(ACTION_NAME, &idem_key, &req_hash).await {
        Ok(Claim::New(token)) => token,
        Ok(Claim::Replay(val)) => {
            return HttpResponse::Ok()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(val);
        }
        Ok(Claim::Conflict) => {
            return HttpResponse::Conflict()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "idempotencyConflict",
                        "message": "idempotency conflict",
                    }
                }));
        }
        Err(e) => {
            tracing::error!("idempotency claim failed: {e}");
            return HttpResponse::InternalServerError()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "serverError",
                        "message": "internal server error",
                    }
                }));
        }
    };

    let service = CaseService::new(
        pool.clone(),
        Arc::new(PgCaseRepository::new(pool.clone())),
        doc_store,
        Arc::new(PgAuditLog::new(pool.clone())),
    );
    let actor = CaseActor::Customer(CustomerActor {
        customer_id: customer_id.to_string(),
    });

    let doc = match service
        .add_document(&actor, case_id, doc_type, &file_bytes, &idempotency_key)
        .await
    {
        Ok(d) => d,
        Err(err) => {
            if let Err(release_err) = idem_store
                .release(ACTION_NAME, &idem_key, &claim_token)
                .await
            {
                tracing::error!("idempotency release failed: {release_err}");
            }
            return map_case_error(err);
        }
    };

    let response_json = serde_json::json!({
        "id": doc.id.to_string(),
        "caseId": doc.case_id.to_string(),
        "documentType": doc.document_type.as_str(),
        "mime": doc.mime,
        "sizeBytes": doc.size_bytes,
        "uploadedAt": doc.uploaded_at.to_rfc3339(),
        "accessClass": doc.access_class.as_str(),
    });

    match pool.begin().await {
        Ok(mut tx) => {
            if let Err(e) = idem_store
                .finish_in(
                    &mut tx,
                    ACTION_NAME,
                    &idem_key,
                    &claim_token,
                    response_json.clone(),
                )
                .await
            {
                tracing::error!("idempotency finish_in failed: {e}");
                let _ = idem_store
                    .release(ACTION_NAME, &idem_key, &claim_token)
                    .await;
                return HttpResponse::InternalServerError()
                    .insert_header((header::CACHE_CONTROL, "no-store"))
                    .json(serde_json::json!({
                        "error": {
                            "code": "serverError",
                            "message": "internal server error",
                        }
                    }));
            }
            if let Err(e) = tx.commit().await {
                tracing::error!("idempotency commit failed: {e}");
                let _ = idem_store
                    .release(ACTION_NAME, &idem_key, &claim_token)
                    .await;
                return HttpResponse::InternalServerError()
                    .insert_header((header::CACHE_CONTROL, "no-store"))
                    .json(serde_json::json!({
                        "error": {
                            "code": "serverError",
                            "message": "internal server error",
                        }
                    }));
            }
        }
        Err(e) => {
            tracing::error!("failed to begin tx for idempotency finish: {e}");
            let _ = idem_store
                .release(ACTION_NAME, &idem_key, &claim_token)
                .await;
            return HttpResponse::InternalServerError()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({
                    "error": {
                        "code": "serverError",
                        "message": "internal server error",
                    }
                }));
        }
    }

    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(response_json)
}
