//! Files routes: uploading a file and reading one back, for the owner and at the staff door.

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_error::AppError;
use serde_json::json;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::{FileService, MAX_UPLOAD_BYTES, StoredFile};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::pages;

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
    tracing::error!(error = %error, "file request failed");
    HttpResponse::InternalServerError()
        .content_type("application/json; charset=utf-8")
        .body(json!({ "error": SERVER_ERROR }).to_string())
}

fn not_found() -> HttpResponse {
    HttpResponse::NotFound()
        .content_type("text/plain")
        .body("Not found")
}

/// The **first** value of a name, as v2's `searchParams.get` reads it: of a repeated name
/// (`?branch=A&branch=B`) the first counts, and a later one does not overwrite it.
fn query(req: &HttpRequest, name: &str) -> Option<String> {
    web::Query::<Vec<(String, String)>>::from_query(req.query_string())
        .ok()
        .and_then(|pairs| {
            pairs
                .into_inner()
                .into_iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value)
        })
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
    actor_id: &str,
    bytes: &[u8],
    no_store_on_refusal: bool,
) -> HttpResponse {
    let Some(files) = req.app_data::<web::Data<FileService>>() else {
        return server_error(&AppError::Internal("no FileService".into()));
    };
    match files
        .upload(branch, actor_id, mime_of(req), bytes, Utc::now())
        .await
    {
        Ok(key) => json_answer(200, json!({ "uploadKey": key }), true),
        Err(AppError::Domain(sentence)) => refuse(400, &sentence, no_store_on_refusal),
        Err(other) => server_error(&other),
    }
}

async fn upload(req: HttpRequest, payload: web::Payload) -> HttpResponse {
    let callers = pages::get_callers(&req).await;
    let now = Utc::now();
    let is_staff = callers.owner_standing().is_none()
        && (callers.staff_standing().is_some() || callers.staff_user.is_some());

    if callers.user.is_none() {
        let allowed = Policy::authorize(None, EntryPath::Upload, &Action::new("upload"), None, now);
        return match allowed {
            Err(refusal) => refuse(refusal.status(), &refusal.to_string(), false),
            Ok(_) => refuse(401, "กรุณาเข้าสู่ระบบ", false),
        };
    }

    if is_staff {
        if !same_origin(&req) {
            return refuse(400, BAD_REQUEST, true);
        }
        let grant = match Policy::authorize(
            callers.user.as_ref(),
            EntryPath::Upload,
            &Action::new("upload"),
            None,
            now,
        ) {
            Ok(g) => g,
            Err(refusal) => return refuse(refusal.status(), &refusal.to_string(), true),
        };
        let bytes = read_upload(payload).await;
        let actor_id = callers
            .user
            .as_ref()
            .map(|u| u.user_id.as_str())
            .unwrap_or("");
        store(&req, &grant.branch, actor_id, &bytes, true).await
    } else {
        if !same_origin(&req) {
            return refuse(400, ORIGIN_REJECTED, false);
        }
        let branch = query(&req, "branch")
            .filter(|b| !b.is_empty())
            .map(BranchId::new);
        let grant = match Policy::authorize(
            callers.user.as_ref(),
            EntryPath::Upload,
            &Action::new("upload"),
            branch.as_ref(),
            now,
        ) {
            Ok(g) => g,
            Err(refusal) => return refuse(refusal.status(), &refusal.to_string(), false),
        };
        let bytes = read_upload(payload).await;
        let actor_id = callers
            .user
            .as_ref()
            .map(|u| u.user_id.as_str())
            .unwrap_or("");
        store(&req, &grant.branch, actor_id, &bytes, false).await
    }
}

async fn staff_upload(req: HttpRequest, payload: web::Payload) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST, true);
    }
    let callers = pages::get_callers(&req).await;
    let now = Utc::now();
    let grant = match Policy::authorize(
        callers.staff_user.as_ref(),
        EntryPath::StaffUpload,
        &Action::new("upload"),
        None,
        now,
    ) {
        Ok(grant) => grant,
        Err(refusal) => return refuse(refusal.status(), &refusal.to_string(), true),
    };
    let bytes = read_upload(payload).await;
    let actor_id = callers
        .staff_user
        .as_ref()
        .map(|u| u.user_id.as_str())
        .unwrap_or("");
    store(&req, &grant.branch, actor_id, &bytes, true).await
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
    let callers = pages::get_callers(&req).await;
    let now = Utc::now();
    let grant = match Policy::authorize(
        callers.user.as_ref(),
        EntryPath::File,
        &Action::new("file"),
        None,
        now,
    ) {
        Ok(grant) => grant,
        Err(PolicyError::SignIn) => {
            return HttpResponse::SeeOther()
                .insert_header((header::LOCATION, format!("{}/login", redirect_origin(&req))))
                .finish();
        }
        Err(_) => return not_found(),
    };
    let Some(files) = req.app_data::<web::Data<FileService>>() else {
        return server_error(&AppError::Internal("no FileService".into()));
    };
    let is_owner = callers.owner_standing().is_some()
        || (callers.user.is_some()
            && callers.staff_standing().is_none()
            && callers.staff_user.is_none());
    if is_owner {
        let actor_id = callers
            .user
            .as_ref()
            .map(|u| u.user_id.as_str())
            .unwrap_or("");
        match files.open_owned(&key, actor_id).await {
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
        }
    } else {
        match files.open_in_branch(&key, &grant.branch).await {
            Ok(Some(stored)) => page_file(stored, None),
            Ok(None) => not_found(),
            Err(e) => server_error(&e),
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
    let callers = pages::get_callers(&req).await;
    let now = Utc::now();
    let grant = match Policy::authorize(
        callers.staff_user.as_ref(),
        EntryPath::StaffFile,
        &Action::new("file"),
        None,
        now,
    ) {
        Ok(grant) => grant,
        Err(refusal) => return door_refusal(refusal.status(), &refusal.to_string()),
    };
    let Some(files) = req.app_data::<web::Data<FileService>>() else {
        return server_error(&AppError::Internal("no FileService".into()));
    };
    match files.open_in_branch(&key, &grant.branch).await {
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
