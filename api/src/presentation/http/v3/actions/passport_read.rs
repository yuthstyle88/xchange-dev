//! `POST /api/v3/compute/passport_read` (design `docs/design/recognize-passport-read.md` §4): the
//! desk's passport image in, Recognize's fields out. Compute-only: it stores nothing. It registers its
//! own handler because `ComputeEndpoint` reads at most 16 KB of JSON (`action.rs:107`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::Utc;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::passport_read::{
    ImageMime, MimeRefusal, PassportImage, PassportReader, ReadError,
};
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::passport_read::PassportReadAnswer;
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::v3::action::{ComputeAction, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::pages;

/// The route's path [required].
pub const PATH: &str = "/api/v3/compute/passport_read";

/// The largest image body, 8 MiB [required] (design §4.2).
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

pub struct PassportReadAction;

impl ComputeAction for PassportReadAction {
    const NAME: &'static str = "passport_read";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Request = ();
    type Answer = PassportReadAnswer;

    async fn compute(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _request: Self::Request,
    ) -> Result<Self::Answer, V3Error> {
        Err(server_error())
    }
}

/// Registers the route with `state.passport_reader`.
pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let reader = state.passport_reader.clone();
    cfg.route(
        PATH,
        web::post().to(move |req: HttpRequest, payload: web::Payload| {
            let reader = reader.clone();
            async move { handle(req, payload, reader).await }
        }),
    );
}

fn query_branch(req: &HttpRequest) -> Option<BranchId> {
    web::Query::<Vec<(String, String)>>::from_query(req.query_string())
        .ok()
        .and_then(|pairs| {
            pairs
                .into_inner()
                .into_iter()
                .find(|(key, _)| key == "branch")
                .map(|(_, value)| value)
        })
        .filter(|b| !b.is_empty())
        .map(BranchId::new)
}

async fn handle(
    req: HttpRequest,
    payload: web::Payload,
    reader: Arc<dyn PassportReader>,
) -> HttpResponse {
    // Step 1: Who is calling (Policy::authorize on PassportReadAction::INHERITS)
    let callers = pages::get_callers(&req).await;
    let branch = query_branch(&req);
    let (entry_path, action_name) = PassportReadAction::INHERITS;
    if let Err(err) = Policy::authorize(
        callers.user.as_ref(),
        entry_path,
        &Action::new(action_name),
        branch.as_ref(),
        Utc::now(),
    ) {
        return V3Error::from(err).error_response();
    }

    // Step 2: Same origin
    if !same_origin(&req) {
        return V3Error::new(
            "originRejected",
            None,
            "ORIGIN_REJECTED",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    }

    // Step 3: Image type (Content-Type)
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mime = match ImageMime::parse(content_type) {
        Ok(mime) => mime,
        Err(MimeRefusal::Pdf) => {
            return refusal(ReadError::PdfNotAccepted).error_response();
        }
        Err(MimeRefusal::Other) => {
            return V3Error::new(
                "imageTypeNotAccepted",
                None,
                "ไม่รองรับชนิดไฟล์รูปภาพนี้",
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
            )
            .error_response();
        }
    };

    // Step 4: Configured? (before reading body)
    if !reader.configured() {
        return refusal(ReadError::NotConfigured).error_response();
    }

    // Step 5: Body (up to MAX_IMAGE_BYTES, non-empty)
    let bytes = match payload.to_bytes_limited(MAX_IMAGE_BYTES).await {
        Ok(Ok(b)) => b,
        Ok(Err(_)) => {
            return refusal(ReadError::ImageInvalid).error_response();
        }
        Err(_) => {
            return refusal(ReadError::ImageTooLarge).error_response();
        }
    };
    if bytes.is_empty() {
        return refusal(ReadError::ImageInvalid).error_response();
    }

    // Step 6: One reader.read call
    let image = PassportImage::new(mime, bytes.to_vec());
    match reader.read(&image).await {
        Ok(read) => {
            let answer = PassportReadAnswer::of(&read);
            HttpResponse::Ok()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(answer)
        }
        Err(err) => {
            let retry_after = match err {
                ReadError::Busy { retry_after_secs } => Some(retry_after_secs),
                _ => None,
            };
            let mut res = refusal(err).error_response();
            if let Some(secs) = retry_after
                && let Ok(val) = header::HeaderValue::from_str(&secs.to_string())
            {
                res.headers_mut().insert(header::RETRY_AFTER, val);
            }
            res
        }
    }
}

fn refusal(err: ReadError) -> V3Error {
    match err {
        ReadError::NotConfigured => V3Error::new(
            "passportReaderNotConfigured",
            None,
            "passport reader is not configured",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        ReadError::Busy { .. } => V3Error::new(
            "passportReaderBusy",
            None,
            "ระบบอ่านหนังสือเดินทางกำลังมีผู้ใช้งานเต็ม กรุณาลองใหม่",
            StatusCode::TOO_MANY_REQUESTS,
        ),
        ReadError::Failed => V3Error::new(
            "passportReaderFailed",
            None,
            "passport reader failed",
            StatusCode::BAD_GATEWAY,
        ),
        ReadError::Timeout => V3Error::new(
            "passportReaderTimeout",
            None,
            "passport reader timed out",
            StatusCode::GATEWAY_TIMEOUT,
        ),
        ReadError::ImageInvalid => V3Error::new(
            "imageInvalid",
            Some("image"),
            "รูปภาพไม่ถูกต้อง",
            StatusCode::BAD_REQUEST,
        ),
        ReadError::ImageTooLarge => V3Error::new(
            "imageTooLarge",
            None,
            "ไฟล์รูปภาพมีขนาดใหญ่เกินไป",
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        ReadError::PdfNotAccepted => V3Error::new(
            "pdfNotAccepted",
            None,
            "ไม่รองรับไฟล์ PDF",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        ReadError::Internal => V3Error::new(
            "serverError",
            None,
            "passport reader internal error",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    }
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::passport_read::export_types(dir)
}
