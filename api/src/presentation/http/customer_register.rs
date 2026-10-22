//! `POST /api/app/register` (#654 CI-7, design `customer-identity-consent.md` §4.4): an Identity user registers as
//! a customer and is signed in. A plain route, not a `CommandAction` (ruling 4 of 2026-10-10 on #654): the caller
//! has no session or branch yet, and the browser never holds an Identity token, so the body carries the login.
//!
//! Body `{identifier, password, displayName, lang: th|en|zh, marketing: [SMS|EMAIL|PUSH]}`. `200 {user, customerId}`
//! with the session cookie set and the old two cleared. Refusals are `{"error": {code, field?, message}}`:
//! - `400 invalidRequest`: cross-site, not JSON, or `identifier`/`password`/`displayName` missing (Identity is not
//!   asked);
//! - `400 validationFailed` naming `lang`, `displayName` or `marketing` (Identity is not asked);
//! - `400 nameTooLong` (`field: displayName`): `displayName` over 100 characters after trim (#4397; Identity is not
//!   asked);
//! - Identity's own refusals (`401 invalidCredentials`, `503 identityUnavailable`, also for a token the verifier
//!   refuses);
//! - `409 alreadyRegistered`, `409 consentTextMissing` (`field: lang`), `409 phoneInUse`.
//!
//! The answer never echoes the identifier, the password, a token or the phone (K8, K10).

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::Utc;
use serde_json::{Value, json};
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xc_auth::application::session::SessionService;
use xc_auth::domain::identity::identity_client::UNAVAILABLE_MESSAGE;
use xc_customer::domain::consent::Channel;
use xc_customer::domain::customer::value_objects::exceeds_display_name_limit;

use crate::application::app_session_claim::claim_session_for;
use crate::application::customer_signup::{SignupError, SignupInput, register_customer};
use crate::bootstrap::AppState;
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::auth::{
    attach_signin_cookies, handle_session_error, refusal_response,
};
use crate::presentation::http::client_address::client_address;
use crate::presentation::http::v3::error::V3Error;

pub const ROUTE: &str = "/api/app/register";

const LANGS: [&str; 3] = ["th", "en", "zh"];

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource(ROUTE)
            .app_data(web::PayloadConfig::new(1 << 16))
            .route(web::post().to(register)),
    );
}

fn invalid_request() -> HttpResponse {
    refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง")
}

fn refused(
    code: &'static str,
    field: Option<&'static str>,
    message: &str,
    status: StatusCode,
) -> HttpResponse {
    let mut response = V3Error::new(code, field, message, status).error_response();
    response.headers_mut().insert(
        actix_web::http::header::CACHE_CONTROL,
        actix_web::http::header::HeaderValue::from_static("no-store"),
    );
    response
}

fn validation_failed(field: &'static str, message: &str) -> HttpResponse {
    refused(
        "validationFailed",
        Some(field),
        message,
        StatusCode::BAD_REQUEST,
    )
}

/// The body as [`SignupInput`], or the refusal, before Identity is asked.
fn input_of(bytes: &[u8]) -> Result<SignupInput, HttpResponse> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| invalid_request())?;
    let text = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or("");
    let (identifier, password, display_name) = (
        text("identifier"),
        text("password"),
        body.get("displayName"),
    );
    let Some(display_name) = display_name.and_then(Value::as_str) else {
        return Err(invalid_request());
    };
    if identifier.is_empty() || password.is_empty() {
        return Err(invalid_request());
    }
    let lang = text("lang");
    if !LANGS.contains(&lang) {
        return Err(validation_failed("lang", "ภาษาต้องเป็น th, en หรือ zh"));
    }
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(validation_failed("displayName", "กรุณากรอกชื่อ"));
    }
    if exceeds_display_name_limit(display_name) {
        return Err(refused(
            "nameTooLong",
            Some("displayName"),
            "ชื่อที่แสดงยาวได้ไม่เกิน 100 ตัวอักษร",
            StatusCode::BAD_REQUEST,
        ));
    }
    let marketing = match body.get("marketing") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut channels = Vec::new();
            for item in items {
                let channel: Channel = item
                    .as_str()
                    .and_then(|c| c.parse().ok())
                    .ok_or_else(|| validation_failed("marketing", "ช่องทางการตลาดไม่ถูกต้อง"))?;
                if !channels.contains(&channel) {
                    channels.push(channel);
                }
            }
            channels
        }
        Some(_) => return Err(validation_failed("marketing", "ช่องทางการตลาดไม่ถูกต้อง")),
    };
    Ok(SignupInput {
        identifier: identifier.to_owned(),
        password: password.to_owned(),
        display_name: display_name.to_owned(),
        lang: lang.to_owned(),
        marketing,
    })
}

async fn register(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
    state: web::Data<AppState>,
) -> HttpResponse {
    if !same_origin(&req) {
        return invalid_request();
    }
    let input = match input_of(&bytes) {
        Ok(input) => input,
        Err(refusal) => return refusal,
    };
    if !session.is_configured() {
        tracing::error!(
            path = ROUTE,
            code = "not_configured",
            "Identity is not configured"
        );
        return refusal_response(503, "identityUnavailable", UNAVAILABLE_MESSAGE);
    }
    let ip = client_address(&req);
    match register_customer(&session, &state.db, input, Some(ip), Utc::now()).await {
        Ok((tail, customer_id)) => {
            // #998 APP-7 (P6): the anonymous app session this browser carries becomes the new customer's.
            if let Some(app) = req
                .cookie(APP_SESSION_COOKIE)
                .and_then(|c| AppSessionId::parse(c.value()))
            {
                claim_session_for(&state.db, &app.hash(), &customer_id, Utc::now()).await;
            }
            let mut response = HttpResponse::Ok();
            attach_signin_cookies(&mut response, &tail.session_token);
            response
                .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({ "user": tail.user, "customerId": customer_id }))
        }
        Err(SignupError::Session(e)) => handle_session_error(e),
        Err(SignupError::AlreadyRegistered) => refused(
            "alreadyRegistered",
            None,
            "บัญชีนี้ลงทะเบียนแล้ว",
            StatusCode::CONFLICT,
        ),
        Err(SignupError::ConsentTextMissing) => refused(
            "consentTextMissing",
            Some("lang"),
            "ยังไม่มีข้อความยินยอมในภาษานี้",
            StatusCode::CONFLICT,
        ),
        Err(SignupError::PhoneInUse) => refused(
            "phoneInUse",
            None,
            "เบอร์โทรนี้ผูกกับลูกค้าอื่นแล้ว",
            StatusCode::CONFLICT,
        ),
        Err(SignupError::Internal(e)) => {
            tracing::error!(error = %e, "customer register failed");
            refused(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        }
    }
}
