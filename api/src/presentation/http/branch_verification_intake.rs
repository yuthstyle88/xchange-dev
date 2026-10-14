//! `POST /api/operations/verification/branch-cases` (design V17, §4.5, ticket #1398 VF-12).
//!
//! At-branch intake for a walk-in customer: opens a case, attaches a document and submits
//! in one transaction, under `verificationBranchCaseOpen`.

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Permission;
use xc_auth::domain::access::role_permissions::RolePermissions;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::pii::identity_type::IdentityType;
use xc_verification::application::branch_intake::{
    BranchDocument, BranchIntake, BranchIntakeError,
};
use xc_verification::domain::case::case_error::CaseError;
use xc_verification::domain::case::reviewer_actor::{ReviewerActor, ReviewerScope};
use xc_verification::infrastructure::pg_case_repository::PgCaseRepository;

use crate::bootstrap::AppState;
use crate::presentation::pages;

pub const ROUTE: &str = "/api/operations/verification/branch-cases";
pub const COMMAND_NAME: &str = "verificationBranchCaseOpen";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchCaseResponse {
    pub id: Uuid,
    pub status: String,
    pub branch_id: Option<String>,
    pub version: i32,
    pub policy_version: i32,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchDocumentResponse {
    pub id: Uuid,
    pub document_type: String,
    pub mime: String,
    pub size_bytes: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchCaseOpenAnswer {
    pub case: BranchCaseResponse,
    pub document: BranchDocumentResponse,
}

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let intake = Arc::new(BranchIntake::new(
        state.db.clone(),
        Arc::new(PgCaseRepository::new(state.db.clone())),
        state.document_store.clone(),
        Arc::new(PgAuditLog::new(state.db.clone())),
    ));
    cfg.app_data(web::Data::new(intake)).service(
        web::resource(ROUTE)
            .app_data(web::PayloadConfig::new(25 * 1024 * 1024))
            .route(web::post().to(open_branch_case)),
    );
}

fn refuse(status: u16, code: &str, message: &str) -> HttpResponse {
    let http_status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
    HttpResponse::build(http_status)
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
            }
        }))
}

fn server_error(cause: impl std::fmt::Display) -> HttpResponse {
    tracing::error!(error = %cause, "branch verification intake error");
    refuse(500, "serverError", "เซิร์ฟเวอร์ขัดข้อง")
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
    customer_id: Option<String>,
    document_type: Option<String>,
    file: Option<Vec<u8>>,
}

fn parse_multipart(boundary: &str, body: &[u8]) -> Result<ParsedMultipart, &'static str> {
    let delim = format!("--{boundary}").into_bytes();
    let mut customer_id = None;
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
        if header_str.contains("name=\"customerId\"") {
            let val = String::from_utf8_lossy(content_bytes).trim().to_string();
            if !val.is_empty() {
                customer_id = Some(val);
            }
        } else if header_str.contains("name=\"documentType\"") {
            let val = String::from_utf8_lossy(content_bytes).trim().to_string();
            if !val.is_empty() {
                document_type = Some(val);
            }
        } else if header_str.contains("name=\"file\"") {
            file = Some(content_bytes.to_vec());
        }
    }

    Ok(ParsedMultipart {
        customer_id,
        document_type,
        file,
    })
}

async fn open_branch_case(req: HttpRequest, payload: web::Payload) -> HttpResponse {
    let idempotency_key = match req
        .headers()
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
    {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return refuse(400, "badRequest", "Missing Idempotency-Key header"),
    };

    let callers = pages::get_callers(&req).await;
    let Some(user) = &callers.user else {
        return refuse(401, "signIn", "กรุณาเข้าสู่ระบบ");
    };

    let now = Utc::now();
    let action = Action::new(COMMAND_NAME);
    let _grant = match Policy::authorize(Some(user), EntryPath::Rpc, &action, None, now) {
        Ok(g) => g,
        Err(e) => {
            let code = match e {
                xc_auth::domain::policy::value_objects::policy_error::PolicyError::RoleNotAllowedHere => {
                    "roleNotAllowedHere"
                }
                xc_auth::domain::policy::value_objects::policy_error::PolicyError::RoleNotAllowed => {
                    "roleNotAllowed"
                }
                xc_auth::domain::policy::value_objects::policy_error::PolicyError::OutOfScope => {
                    "outOfScope"
                }
                xc_auth::domain::policy::value_objects::policy_error::PolicyError::SignIn => {
                    "signIn"
                }
                _ => "roleNotAllowed",
            };
            let status = match e {
                xc_auth::domain::policy::value_objects::policy_error::PolicyError::SignIn => 401,
                _ => 403,
            };
            return refuse(status, code, &e.to_string());
        }
    };

    let user_id = match Uuid::parse_str(&user.user_id) {
        Ok(u) => u,
        Err(_) => return refuse(403, "outOfScope", "invalid user_id"),
    };
    let org_id = match Uuid::parse_str(&user.organization_id) {
        Ok(o) => o,
        Err(_) => return refuse(403, "outOfScope", "invalid organization_id"),
    };

    let reviewer_role = user.roles.iter().find(|r| {
        r.valid_from <= now
            && r.valid_to.is_none_or(|to| now < to)
            && RolePermissions::of(r.role).contains(&Permission::VerificationReview)
    });

    let reviewer_scope = match reviewer_role {
        Some(r) => match &r.branch {
            Some(b) => ReviewerScope::Branch {
                organization_id: org_id,
                branch_id: b.clone(),
            },
            None => ReviewerScope::Organization {
                organization_id: org_id,
            },
        },
        None => return refuse(403, "roleNotAllowed", "Role has no VerificationReview"),
    };

    let reviewer = ReviewerActor {
        user_id,
        scope: reviewer_scope,
    };

    // Extract boundary
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let boundary = content_type.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix("boundary=")
            .map(|rest| rest.trim().trim_matches('"'))
    });

    let Some(boundary) = boundary else {
        return refuse(400, "badRequest", "Missing multipart boundary");
    };

    let body_bytes = match payload.to_bytes_limited(25 * 1024 * 1024).await {
        Ok(Ok(b)) => b.to_vec(),
        _ => return refuse(400, "validationFailed", "ไฟล์ใหญ่เกิน 15 MB"),
    };

    let parsed = match parse_multipart(boundary, &body_bytes) {
        Ok(p) => p,
        Err(_) => return refuse(400, "badRequest", "Invalid multipart body"),
    };

    let (customer_id, document_type_str, file_bytes) =
        match (parsed.customer_id, parsed.document_type, parsed.file) {
            (Some(c), Some(d), Some(f)) => (c, d, f),
            _ => return refuse(400, "badRequest", "Missing required multipart fields"),
        };

    let document_type = match document_type_str.to_uppercase().as_str() {
        "PASSPORT" => IdentityType::Passport,
        "NATIONAL_ID" => IdentityType::NationalId,
        "PHONE" => IdentityType::Phone,
        "EMAIL" => IdentityType::Email,
        _ => return refuse(400, "documentTypeNotAccepted", "ชนิดเอกสารไม่ถูกต้อง"),
    };

    let intake: Arc<BranchIntake> = if let Some(i) = req.app_data::<web::Data<Arc<BranchIntake>>>()
    {
        i.get_ref().clone()
    } else {
        return server_error("BranchIntake is not wired in app_data");
    };

    let branch_doc = BranchDocument::new(document_type, file_bytes, idempotency_key);
    match intake.open(&reviewer, &customer_id, branch_doc).await {
        Ok((case, doc)) => {
            let answer = BranchCaseOpenAnswer {
                case: BranchCaseResponse {
                    id: case.id,
                    status: case.status.as_str().to_string(),
                    branch_id: case.branch_id.map(|b| b.as_str().to_string()),
                    version: case.version,
                    policy_version: case.policy_version,
                    provider: case.provider.as_str().to_string(),
                },
                document: BranchDocumentResponse {
                    id: doc.id,
                    document_type: doc.document_type.as_str().to_string(),
                    mime: doc.mime,
                    size_bytes: doc.size_bytes,
                },
            };
            HttpResponse::Ok()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(answer)
        }
        Err(err) => match err {
            BranchIntakeError::BranchReviewNotAllowed => {
                refuse(400, "branchReviewNotAllowed", "branchReviewNotAllowed")
            }
            BranchIntakeError::OutOfScope => refuse(403, "outOfScope", "outOfScope"),
            BranchIntakeError::DocumentTypeNotAccepted => {
                refuse(400, "documentTypeNotAccepted", "documentTypeNotAccepted")
            }
            BranchIntakeError::NoVerificationPolicy => {
                refuse(400, "noVerificationPolicy", "noVerificationPolicy")
            }
            BranchIntakeError::CaseLocked => refuse(400, "caseLocked", "caseLocked"),
            BranchIntakeError::IdempotencyConflict => {
                refuse(409, "idempotencyConflict", "idempotencyConflict")
            }
            BranchIntakeError::File(msg) => refuse(400, "validationFailed", msg),
            BranchIntakeError::NotFound => refuse(404, "notFound", "notFound"),
            BranchIntakeError::Case(CaseError::Store(e)) => server_error(e),
            BranchIntakeError::Case(CaseError::Documents(e)) => server_error(e),
            BranchIntakeError::Case(CaseError::OutOfScope) => {
                refuse(403, "outOfScope", "outOfScope")
            }
            BranchIntakeError::Case(CaseError::IdempotencyConflict) => {
                refuse(409, "idempotencyConflict", "idempotencyConflict")
            }
            BranchIntakeError::Case(CaseError::File(msg)) => refuse(400, "validationFailed", msg),
            BranchIntakeError::Case(e) => refuse(400, e.code(), &e.to_string()),
            BranchIntakeError::Store(e) => server_error(e),
            BranchIntakeError::Documents(e) => server_error(e),
        },
    }
}
