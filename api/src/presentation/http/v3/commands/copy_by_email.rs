//! `POST /api/app/copy-by-email` (`copyByEmail`, design `kiosk-shared-phone.md` §4.2 D21, §4.4
//! B19–B23): a signed-in member has the copy of their own booking or web request e-mailed to their
//! verified e-mail. `CustomerSelf` (policy area `copy_by_email.rs`).
//!
//! A plain route, not a `CommandAction`: the link in the e-mail opens on the request's own origin,
//! which only the request carries (`Origin`, already checked by `same_origin`), and the send is
//! made idempotent by Notify's key, not by a stored command row.
//!
//! `[required]` wire: body `{subject: "booking" | "webRequest", id, qr}`; answers `200 {sent:
//! true}` or `200 {sent: true, again: true}`; `404 notFound`, `409 noVerifiedEmail`, `503
//! emailUnavailable` in the V3 error envelope (`{error: {code, …}}`); an unknown `subject`, or a
//! missing/empty field ⇒ `400 validationFailed` naming the field. The body has no address field:
//! the recipient is never the caller's choice.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::Utc;
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::domain::email_sender::EMAIL_UNAVAILABLE;

use crate::application::copy_by_email::booking_copy::BookingCopy;
use crate::application::copy_by_email::link::CopyLink;
use crate::application::copy_by_email::member_email::PgMemberEmails;
use crate::application::copy_by_email::service::CopyByEmailService;
use crate::application::copy_by_email::subject::{CopySubject, CopySubjectKind};
use crate::application::copy_by_email::web_request_copy::WebRequestCopy;
use crate::bootstrap::AppState;
use crate::infrastructure::notify::notify_email_sender::NotifyEmailSender;
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::v3::command::{role_not_allowed_here, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::authenticate_and_authorize;

pub const ROUTE: &str = "/api/app/copy-by-email";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "copy_by_email.ts")]
pub struct CopyByEmailInput {
    /// `booking` or `webRequest`.
    pub subject: String,
    pub id: String,
    /// The QR in the clear, as the member's own page holds it (the part after `#`).
    pub qr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "copy_by_email.ts")]
pub struct CopyByEmailOutput {
    pub sent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub again: Option<bool>,
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone()))
        .route(ROUTE, web::post().to(handle));
}

fn validation_failed(field: &'static str) -> HttpResponse {
    V3Error::new(
        "validationFailed",
        Some(field),
        "ข้อมูลไม่ถูกต้อง",
        StatusCode::BAD_REQUEST,
    )
    .error_response()
}

fn map_error(err: AppError) -> HttpResponse {
    match err {
        AppError::NotFound { .. } => {
            V3Error::new("notFound", None, "ไม่พบรายการ", StatusCode::NOT_FOUND).error_response()
        }
        AppError::Conflict(ref code) if code == "noVerifiedEmail" => V3Error::new(
            "noVerifiedEmail",
            None,
            "ยังไม่มีอีเมลที่ยืนยันแล้ว",
            StatusCode::CONFLICT,
        )
        .error_response(),
        AppError::Domain(ref code) if code == EMAIL_UNAVAILABLE => V3Error::new(
            "emailUnavailable",
            None,
            EMAIL_UNAVAILABLE,
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .error_response(),
        _ => server_error().error_response(),
    }
}

async fn handle(req: HttpRequest, state: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    if !same_origin(&req) {
        return V3Error::new(
            "originRejected",
            None,
            "ORIGIN_REJECTED",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    }

    let who = match authenticate_and_authorize(&req, (EntryPath::Rpc, "copyByEmail")).await {
        Ok(who) => who,
        Err(resp) => return resp,
    };
    let Some(customer_id) = who.customer_id.as_deref() else {
        return role_not_allowed_here().error_response();
    };

    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
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
    let Some(obj) = value.as_object() else {
        return V3Error::new(
            "invalidRequest",
            None,
            "คำขอไม่ถูกต้อง",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    };

    let Some(kind) = obj
        .get("subject")
        .and_then(Value::as_str)
        .and_then(CopySubjectKind::parse)
    else {
        return validation_failed("subject");
    };
    let Some(id) = obj
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return validation_failed("id");
    };
    let Some(qr) = obj
        .get("qr")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return validation_failed("qr");
    };

    let Ok(keys) = xc_customer::get_pii_keys() else {
        return server_error().error_response();
    };
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    let pool = state.db.clone();
    let subjects: Vec<Arc<dyn CopySubject>> = vec![
        Arc::new(BookingCopy::new(pool.clone())),
        Arc::new(WebRequestCopy::new(pool.clone())),
    ];
    let service = CopyByEmailService::new(
        subjects,
        Arc::new(PgMemberEmails::new(pool, keys.clone())),
        Arc::new(NotifyEmailSender::from_env()),
        CopyLink::from_pii(&keys),
        origin,
    );

    match service.send(customer_id, kind, id, qr, Utc::now()).await {
        Ok(sent) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(CopyByEmailOutput {
                sent: true,
                again: sent.again.then_some(true),
            }),
        Err(err) => map_error(err),
    }
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CopyByEmailInput::export_all(&cfg)?;
    CopyByEmailOutput::export_all(&cfg)?;
    Ok(())
}
