//! Files routes: uploading a file and reading one back, for the owner and at the staff door.

use std::collections::HashMap;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_error::AppError;
use serde_json::json;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::{FileService, MAX_UPLOAD_BYTES, StoredFile};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::pages::{self, SessionReader, Sessions};

const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const NO_FILE: &str = "ไม่พบไฟล์";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";
const SANDBOX: &str =
    "sandbox allow-modals; default-src 'none'; style-src 'unsafe-inline'; img-src data:";

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let files = FileService::new(state.db.clone(), state.settings.files.dir.clone());
    cfg.app_data(web::Data::new(files));
    cfg.service(
        web::resource("/api/upload")
            .route(web::post().to(upload))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/api/staff/upload")
            .route(web::post().to(staff_upload))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/files/{key}")
            .route(web::get().to(file))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/api/staff/file/{key}")
            .route(web::get().to(staff_file))
            .default_service(web::to(pages::fallback)),
    );
}

async fn sessions(req: &HttpRequest) -> Sessions {
    match req.app_data::<web::Data<dyn SessionReader>>() {
        Some(reader) => reader.sessions(req).await,
        None => Sessions {
            owner: None,
            staff: None,
        },
    }
}

fn redirect_origin(req: &HttpRequest) -> String {
    let https = req
        .headers()
        .get("x-forwarded-proto")
        .is_some_and(|v| v.as_bytes() == b"https");
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    format!("{}://{}", if https { "https" } else { "http" }, host)
}

fn json_answer(status: u16, body: serde_json::Value, no_store: bool) -> HttpResponse {
    let mut response = HttpResponse::build(StatusCode::from_u16(status).unwrap());
    if no_store {
        response.insert_header((header::CACHE_CONTROL, "no-store"));
    }
    response.json(body)
}

fn refuse(status: u16, sentence: &str, no_store: bool) -> HttpResponse {
    json_answer(status, json!({ "error": sentence }), no_store)
}

fn server_error(error: &AppError) -> HttpResponse {
    tracing::error!(error = ?error, "file request failed");
    HttpResponse::InternalServerError()
        .content_type("application/json; charset=utf-8")
        .body(json!({ "error": SERVER_ERROR }).to_string())
}

fn not_found() -> HttpResponse {
    HttpResponse::NotFound()
        .content_type("text/plain")
        .body("Not found")
}

fn query(req: &HttpRequest, name: &str) -> Option<String> {
    web::Query::<HashMap<String, String>>::from_query(req.query_string())
        .ok()
        .and_then(|q| q.get(name).cloned())
}

fn mime_of(req: &HttpRequest) -> &str {
    req.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// The body, read no further than the limit. A longer one is handed on as one byte more than
/// the limit, so that the service refuses it in its own order (after the branch and the type).
async fn read_upload(payload: web::Payload) -> Vec<u8> {
    match payload.to_bytes_limited(MAX_UPLOAD_BYTES).await {
        Ok(Ok(bytes)) => bytes.to_vec(),
        _ => vec![0; MAX_UPLOAD_BYTES + 1],
    }
}

async fn store(
    req: &HttpRequest,
    branch: &BranchId,
    actor: &Actor,
    bytes: &[u8],
    no_store_on_refusal: bool,
) -> HttpResponse {
    let Some(files) = req.app_data::<web::Data<FileService>>() else {
        return server_error(&AppError::Internal("no FileService".into()));
    };
    match files
        .upload(branch, actor.id(), mime_of(req), bytes, Utc::now())
        .await
    {
        Ok(key) => json_answer(200, json!({ "uploadKey": key }), true),
        Err(AppError::Domain(sentence)) => refuse(400, &sentence, no_store_on_refusal),
        Err(other) => server_error(&other),
    }
}

async fn upload(req: HttpRequest, payload: web::Payload) -> HttpResponse {
    let bytes = read_upload(payload).await;
    let Sessions { owner, staff } = sessions(&req).await;
    let actor = owner.or(staff);
    let allowed = Policy::allow(actor.as_ref(), EntryPath::Upload, &Action::new("upload"));
    let Some(actor) = actor else {
        return match allowed {
            Err(refusal) => refuse(refusal.status(), &refusal.to_string(), false),
            Ok(()) => refuse(401, "กรุณาเข้าสู่ระบบ", false),
        };
    };
    match actor.kind() {
        ActorKind::Owner => {
            if !same_origin(&req) {
                return refuse(400, ORIGIN_REJECTED, false);
            }
            let branch = query(&req, "branch")
                .filter(|b| !b.is_empty())
                .unwrap_or_else(|| "MAIN".to_owned());
            store(&req, &BranchId::new(branch), &actor, &bytes, false).await
        }
        ActorKind::Staff => {
            if !same_origin(&req) {
                return refuse(400, BAD_REQUEST, true);
            }
            if let Err(refusal) = allowed {
                return refuse(refusal.status(), &refusal.to_string(), true);
            }
            let Some(branch) = actor.branch_id().cloned() else {
                return refuse(400, BAD_REQUEST, true);
            };
            store(&req, &branch, &actor, &bytes, true).await
        }
    }
}

async fn staff_upload(req: HttpRequest, payload: web::Payload) -> HttpResponse {
    let bytes = read_upload(payload).await;
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST, true);
    }
    let staff = sessions(&req).await.staff;
    if let Err(refusal) = Policy::allow(
        staff.as_ref(),
        EntryPath::StaffUpload,
        &Action::new("upload"),
    ) {
        return refuse(refusal.status(), &refusal.to_string(), true);
    }
    let Some((actor, branch)) = staff.and_then(|a| a.branch_id().cloned().map(|b| (a, b))) else {
        return refuse(401, "EMPLOYEE_AUTH_REQUIRED", true);
    };
    store(&req, &branch, &actor, &bytes, true).await
}

fn ending(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/html" => "html",
        _ => "bin",
    }
}

fn page_file(stored: StoredFile, disposition: Option<String>) -> HttpResponse {
    let mut response = HttpResponse::Ok();
    response
        .insert_header((header::CONTENT_TYPE, stored.mime.clone()))
        .insert_header((header::CACHE_CONTROL, "private, no-store"))
        .insert_header((header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .insert_header((header::CONTENT_SECURITY_POLICY, SANDBOX));
    if let Some(disposition) = disposition {
        response.insert_header((header::CONTENT_DISPOSITION, disposition));
    }
    response.body(stored.bytes)
}

async fn file(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let key = path.into_inner();
    let Sessions { owner, staff } = sessions(&req).await;
    let Some(actor) = owner.or(staff) else {
        return HttpResponse::SeeOther()
            .insert_header((header::LOCATION, format!("{}/login", redirect_origin(&req))))
            .finish();
    };
    let Some(files) = req.app_data::<web::Data<FileService>>() else {
        return server_error(&AppError::Internal("no FileService".into()));
    };
    match actor.kind() {
        ActorKind::Owner => match files.open_owned(&key, actor.id()).await {
            Ok(Some(stored)) => {
                let disposition = if query(&req, "download").as_deref() == Some("1") {
                    format!("attachment; filename=\"{key}.{}\"", ending(&stored.mime))
                } else {
                    "inline".to_owned()
                };
                page_file(stored, Some(disposition))
            }
            Ok(None) => not_found(),
            Err(e) => server_error(&e),
        },
        ActorKind::Staff => {
            if Policy::allow(Some(&actor), EntryPath::File, &Action::new("file")).is_err() {
                return not_found();
            }
            let Some(branch) = actor.branch_id() else {
                return not_found();
            };
            match files.open_in_branch(&key, branch).await {
                Ok(Some(stored)) => page_file(stored, None),
                Ok(None) => not_found(),
                Err(e) => server_error(&e),
            }
        }
    }
}

fn door_refusal(status: u16, sentence: &str) -> HttpResponse {
    HttpResponse::build(StatusCode::from_u16(status).unwrap())
        .insert_header((header::CACHE_CONTROL, "private, no-store"))
        .insert_header((header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .insert_header((header::CONTENT_DISPOSITION, "attachment"))
        .json(json!({ "error": sentence }))
}

async fn staff_file(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let key = path.into_inner();
    let staff = sessions(&req).await.staff;
    if let Err(refusal) = Policy::allow(staff.as_ref(), EntryPath::StaffFile, &Action::new("file"))
    {
        return door_refusal(refusal.status(), &refusal.to_string());
    }
    let Some(branch) = staff.as_ref().and_then(|a| a.branch_id()) else {
        return door_refusal(401, "EMPLOYEE_AUTH_REQUIRED");
    };
    let Some(files) = req.app_data::<web::Data<FileService>>() else {
        return server_error(&AppError::Internal("no FileService".into()));
    };
    match files.open_in_branch(&key, branch).await {
        Ok(Some(stored)) => {
            let disposition = if stored.mime == "text/html" {
                "attachment; filename=\"receipt.html\""
            } else {
                "inline"
            };
            HttpResponse::Ok()
                .insert_header((header::CONTENT_TYPE, stored.mime.clone()))
                .insert_header((header::CACHE_CONTROL, "private, no-store"))
                .insert_header((header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
                .insert_header((header::CONTENT_DISPOSITION, disposition))
                .body(stored.bytes)
        }
        Ok(None) => door_refusal(400, NO_FILE),
        Err(e) => server_error(&e),
    }
}
