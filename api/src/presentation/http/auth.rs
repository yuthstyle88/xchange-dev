//! Auth routes: the Identity BFF routes under `/api/auth/*`. OR-7.m w2 (#1027): the v2 password
//! doors (`/api/auth`, `/api/staff/{login,logout,password,me}`) are gone.

use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::application::session::session_service::SwitchBranchError;
use xc_auth::application::session::{SESSION_COOKIE, SessionError, SessionService, sha256_hex};
use xc_auth::domain::identity::identity_client::{
    IdentityClient, IdentityError, UNAVAILABLE_MESSAGE,
};
use xc_auth::domain::token::TokenVerifier;
use xc_auth::infrastructure::identity::HttpIdentityClient;
use xc_auth::infrastructure::token::http_jwks_source::HttpJwksSource;
use xc_auth::infrastructure::token::jwks_verifier::JwksTokenVerifier;
use xc_kernel::branch_tx::BranchTx;

use crate::application::customer_binder::BindError;
use crate::application::customer_otp_sign_in::{CustomerOtpSignIn, OtpRegister, OtpSignInError};
use crate::application::otp_challenge_seal::{OTP_COOKIE, OtpChallengeSeal, OtpChannel};
use crate::bootstrap::AppState;
use crate::infrastructure::branch_tablets::TABLET_COOKIE;
use crate::infrastructure::branch_tablets::tablet_service::BranchTabletService;
use crate::presentation::extractors::actor::{
    AccountSessionReader, cookie, failure, same_origin, server_error,
};
use crate::presentation::extractors::session_actor::actor_of_user;
use crate::presentation::http::client_address::{CLIENT_ADDRESS_HEADER, client_address};
use crate::presentation::http::v3::me::me_of;
use crate::presentation::pages::SessionReader;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let reader: Arc<dyn SessionReader> = Arc::new(AccountSessionReader);
    cfg.app_data(web::Data::from(reader));

    let (identity_client, token_verifier, client_secret) =
        if let Some(ref id_settings) = state.settings.identity {
            let client: Arc<dyn IdentityClient> = Arc::new(HttpIdentityClient::new(
                id_settings.clone(),
                CLIENT_ADDRESS_HEADER,
            ));
            let jwks_url = format!(
                "{}/.well-known/jwks.json",
                id_settings.base_url.trim_end_matches('/')
            );
            let jwks_source = Arc::new(HttpJwksSource::new(jwks_url));
            let verifier: Arc<dyn TokenVerifier> = Arc::new(JwksTokenVerifier::new(
                jwks_source,
                &id_settings.issuer,
                &id_settings.audience,
            ));
            (
                Some(client),
                Some(verifier),
                Some(id_settings.client_secret.clone()),
            )
        } else {
            (None, None, None)
        };

    let session_service = Arc::new(SessionService::new(
        state.db.clone(),
        identity_client,
        token_verifier,
        client_secret,
    ));
    cfg.app_data(web::Data::from(session_service));

    let body = || web::PayloadConfig::new(1 << 20);

    // Identity BFF routes (POST with JSON)
    cfg.service(
        web::resource("/api/auth/login")
            .app_data(body())
            .route(web::post().to(v3_login)),
    );
    cfg.service(
        web::resource("/api/auth/otp/request")
            .app_data(body())
            .route(web::post().to(v3_otp_request)),
    );
    cfg.service(
        web::resource("/api/auth/otp/verify")
            .app_data(body())
            .route(web::post().to(v3_otp_verify)),
    );
    cfg.service(
        web::resource("/api/auth/session")
            .app_data(body())
            .route(web::post().to(v3_session)),
    );
    cfg.service(
        web::resource("/api/auth/session/branch")
            .app_data(body())
            .route(web::post().to(v3_session_branch)),
    );
    cfg.service(
        web::resource("/api/auth/refresh")
            .app_data(body())
            .route(web::post().to(v3_refresh)),
    );
    cfg.service(
        web::resource("/api/auth/logout")
            .app_data(body())
            .route(web::post().to(v3_logout)),
    );
    cfg.service(
        web::resource("/api/auth/password")
            .app_data(body())
            .route(web::post().to(v3_password)),
    );
}

pub(crate) fn refusal_response(status: u16, code: &str, message: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
        .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .json(json!({
            "error": { "code": code, "message": message }
        }))
}

pub(crate) fn handle_session_error(err: SessionError) -> HttpResponse {
    if let SessionError::Identity(IdentityError::Unavailable { path, status, code }) = &err {
        tracing::error!(
            path = %path,
            status = status,
            code = %code,
            "Identity unavailable"
        );
    }
    let (status, code, msg) = err.refusal();
    refusal_response(status, code, &msg)
}

pub(crate) fn attach_signin_cookies(
    builder: &mut actix_web::HttpResponseBuilder,
    session_token: &str,
) {
    let session_cookie = format!(
        "__Host-xc_session={session_token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=43200"
    );

    builder.append_header((actix_web::http::header::SET_COOKIE, session_cookie));
}

fn attach_logout_cookies(builder: &mut actix_web::HttpResponseBuilder) {
    let session_cookie = "__Host-xc_session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";

    builder.append_header((actix_web::http::header::SET_COOKIE, session_cookie));
}

fn attach_clear_session_cookie(builder: &mut actix_web::HttpResponseBuilder) {
    let session_cookie = "__Host-xc_session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    builder.append_header((actix_web::http::header::SET_COOKIE, session_cookie));
}

async fn v3_login(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    if !session.is_configured() {
        tracing::error!(
            path = "/api/auth/login",
            status = 0,
            code = "not_configured",
            "Identity is not configured"
        );
        return refusal_response(503, "identityUnavailable", UNAVAILABLE_MESSAGE);
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"),
    };
    let identifier = body
        .get("identifier")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    if identifier.is_empty() || password.is_empty() {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let ip = client_address(&req);
    let now = Utc::now();
    match session.login(identifier, password, Some(ip), now).await {
        Ok(tail) => {
            let mut resp = HttpResponse::Ok();
            attach_signin_cookies(&mut resp, &tail.session_token);
            resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({ "user": tail.user }))
        }
        Err(err) => handle_session_error(err),
    }
}

async fn v3_otp_request(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    if !session.is_configured() {
        tracing::error!(
            path = "/api/auth/otp/request",
            status = 0,
            code = "not_configured",
            "Identity is not configured"
        );
        return refusal_response(503, "identityUnavailable", UNAVAILABLE_MESSAGE);
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"),
    };
    let destination = body
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let channel = body.get("channel").and_then(|v| v.as_str()).unwrap_or("");
    let Some(otp_channel) = OtpChannel::parse(channel) else {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    };
    if destination.is_empty() {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let ip = client_address(&req);
    match session.otp_request(destination, channel, Some(ip)).await {
        Ok(resp) => {
            let mut response = HttpResponse::Ok();
            let id_type = match otp_channel {
                OtpChannel::Sms => xc_kernel::pii::IdentityType::Phone,
                OtpChannel::Email => xc_kernel::pii::IdentityType::Email,
            };
            if let (Ok(norm_dest), Ok(keys)) = (
                xc_kernel::pii::normalize(id_type, destination, None),
                xc_customer::get_pii_keys(),
            ) {
                let sealed = OtpChallengeSeal {
                    channel: otp_channel,
                    destination: norm_dest,
                    challenge_id: resp.challenge_id.clone(),
                }
                .seal(&keys);
                let now_secs = Utc::now().timestamp().max(0) as u64;
                let max_age = resp.expires_at.saturating_sub(now_secs).min(600);
                let otp_cookie = format!(
                    "{OTP_COOKIE}={sealed}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}"
                );
                response.append_header((actix_web::http::header::SET_COOKIE, otp_cookie));
            }
            response
                .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({
                    "challengeId": resp.challenge_id,
                    "expiresAt": resp.expires_at,
                }))
        }
        Err(err) => handle_session_error(err),
    }
}

fn clear_otp_cookie(mut resp: HttpResponse) -> HttpResponse {
    let clear = format!("{OTP_COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0");
    resp.headers_mut().append(
        actix_web::http::header::SET_COOKIE,
        actix_web::http::header::HeaderValue::from_str(&clear).unwrap(),
    );
    resp
}

fn parse_otp_register(body: &Value) -> Result<Option<OtpRegister>, HttpResponse> {
    let Some(reg) = body.get("register") else {
        return Ok(None);
    };
    if reg.is_null() {
        return Ok(None);
    }
    let Some(obj) = reg.as_object() else {
        return Err(refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"));
    };
    let Some(display_name) = obj.get("displayName").and_then(Value::as_str) else {
        return Err(refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"));
    };
    let lang = obj.get("lang").and_then(Value::as_str).unwrap_or("");
    if !["th", "en", "zh"].contains(&lang) {
        return Err(validation_response("lang", "ภาษาต้องเป็น th, en หรือ zh"));
    }
    if display_name.trim().is_empty() {
        return Err(validation_response("displayName", "กรุณากรอกชื่อ"));
    }
    let marketing = match obj.get("marketing") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut channels = Vec::new();
            for item in items {
                let channel: xc_customer::domain::consent::Channel = item
                    .as_str()
                    .and_then(|c| c.parse().ok())
                    .ok_or_else(|| validation_response("marketing", "ช่องทางการตลาดไม่ถูกต้อง"))?;
                if !channels.contains(&channel) {
                    channels.push(channel);
                }
            }
            channels
        }
        Some(_) => return Err(validation_response("marketing", "ช่องทางการตลาดไม่ถูกต้อง")),
    };
    Ok(Some(OtpRegister {
        display_name: display_name.trim().to_owned(),
        lang: lang.to_owned(),
        marketing,
    }))
}

async fn v3_otp_verify(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
    state: web::Data<AppState>,
) -> HttpResponse {
    clear_otp_cookie(v3_otp_verify_inner(req, bytes, session, state).await)
}

async fn v3_otp_verify_inner(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
    state: web::Data<AppState>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    if !session.is_configured() {
        tracing::error!(
            path = "/api/auth/otp/verify",
            status = 0,
            code = "not_configured",
            "Identity is not configured"
        );
        return refusal_response(503, "identityUnavailable", UNAVAILABLE_MESSAGE);
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"),
    };
    let challenge_id = body
        .get("challengeId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");
    if challenge_id.is_empty() || code.is_empty() {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let register = match parse_otp_register(&body) {
        Ok(reg) => reg,
        Err(resp) => return resp,
    };
    let seal = xc_customer::get_pii_keys()
        .ok()
        .and_then(|keys| OtpChallengeSeal::open(&cookie(&req, OTP_COOKIE), &keys));
    let ip = client_address(&req);
    let now = Utc::now();
    match CustomerOtpSignIn::new(&session, &state.db)
        .sign_in(challenge_id, code, seal.as_ref(), register, Some(ip), now)
        .await
    {
        Ok(signed_in) => {
            // #998 APP-7 (P6): the anonymous app session this browser carries becomes the customer's.
            if let Some(app) = req
                .cookie(xc_app::domain::app_session::APP_SESSION_COOKIE)
                .and_then(|c| xc_app::domain::app_session::AppSessionId::parse(c.value()))
                && let Some(ref cid) = signed_in.customer_id
            {
                crate::application::app_session_claim::claim_session_for(
                    &state.db,
                    &app.hash(),
                    cid,
                    now,
                )
                .await;
            }
            let tablet_token = cookie(&req, TABLET_COOKIE);
            if !tablet_token.is_empty()
                && let Err(e) = branch_tablets(&state.db)
                    .bind_on_sign_in(
                        &tablet_token,
                        &sha256_hex(&signed_in.tail.session_token),
                        &signed_in.tail.user.id,
                        now,
                    )
                    .await
            {
                tracing::error!(error = %e, "branch tablet sign-in bind failed");
                if let Err(logout_err) = session
                    .logout(&signed_in.tail.session_token, Some(ip))
                    .await
                {
                    tracing::error!(
                        error = ?logout_err,
                        "session logout after branch tablet bind failure failed"
                    );
                }
                return refusal_response(500, "serverError", "เซิร์ฟเวอร์ขัดข้อง");
            }
            let mut resp = HttpResponse::Ok();
            attach_signin_cookies(&mut resp, &signed_in.tail.session_token);
            resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({
                    "user": signed_in.tail.user,
                    "customerId": signed_in.customer_id,
                    "registered": signed_in.registered,
                }))
        }
        Err(OtpSignInError::NotRegistered) => {
            refusal_response(401, "notRegistered", "ยังไม่มีบัญชี กรุณาสมัครสมาชิก")
        }
        Err(OtpSignInError::Bind(BindError::AlreadyRegistered)) => {
            refusal_response(409, "alreadyRegistered", "บัญชีนี้ลงทะเบียนแล้ว")
        }
        Err(OtpSignInError::Bind(BindError::ConsentTextMissing)) => HttpResponse::Conflict()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({
                "error": {
                    "code": "consentTextMissing",
                    "field": "lang",
                    "message": "ยังไม่มีข้อความยินยอมในภาษานี้",
                }
            })),
        Err(OtpSignInError::Bind(BindError::PhoneInUse)) => {
            refusal_response(409, "phoneInUse", "เบอร์โทรนี้ผูกกับลูกค้าอื่นแล้ว")
        }
        Err(OtpSignInError::Bind(BindError::EmailInUse)) => {
            refusal_response(409, "emailInUse", "อีเมลนี้ผูกกับลูกค้าอื่นแล้ว")
        }
        Err(OtpSignInError::Bind(BindError::Internal(e))) => {
            tracing::error!(error = %e, "customer otp sign-in failed");
            refusal_response(500, "serverError", "เซิร์ฟเวอร์ขัดข้อง")
        }
        Err(OtpSignInError::Session(err)) => handle_session_error(err),
    }
}

async fn v3_session(req: HttpRequest, session: web::Data<SessionService>) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let token = cookie(&req, SESSION_COOKIE);
    if token.is_empty() {
        return HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({ "user": null }));
    }
    let now = Utc::now();
    match session.session_check(&token, now).await {
        Ok(Some(user)) => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({ "user": user })),
        _ => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({ "user": null })),
    }
}

/// `POST /api/auth/session/branch` (SB-1 §14.3): `{"branch": "<id>"}` moves this session to that branch and
/// answers the new `Me` as the whole body. `400 invalidRequest` malformed or cross-site; `401 signIn` no session;
/// `403 outOfScope` a branch outside S, with nothing written.
async fn v3_session_branch(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
    state: web::Data<AppState>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"),
    };
    let Some(branch) = body.get("branch").and_then(Value::as_str) else {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    };
    let token = cookie(&req, SESSION_COOKIE);
    if token.is_empty() {
        return refusal_response(401, "signIn", "กรุณาเข้าสู่ระบบ");
    }
    let now = Utc::now();
    let active = match session.switch_branch(&token, branch, now).await {
        Ok(active) => active,
        Err(SwitchBranchError::SignIn) => {
            return refusal_response(401, "signIn", "กรุณาเข้าสู่ระบบ");
        }
        Err(SwitchBranchError::OutOfScope) => {
            return refusal_response(403, "outOfScope", "ไม่มีสิทธิ์ในสาขานี้");
        }
        Err(SwitchBranchError::Database(e)) => {
            tracing::error!(error = %e, "session branch switch failed");
            return server_error();
        }
    };
    let s = active.switchable(now);
    let chosen = xc_kernel::ids::BranchId::new(branch);
    // #654: the actor with its customer link, in the branch just chosen.
    let Some(access_actor) = actor_of_user(&state.db, &active.user, active.roles).await else {
        return server_error();
    };
    let access_actor = access_actor.with_branch(Some(chosen.clone()));
    // The same `Me` the `me` view answers (#1030: from the Blueprint actor itself).
    match me_of(&state.db, &access_actor, &chosen, Some(s)).await {
        Ok(me) => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(me),
        Err(e) => failure(e),
    }
}

fn branch_tablets(pool: &sqlx::PgPool) -> BranchTabletService {
    BranchTabletService::new(BranchTx::new(Transactor::new(pool.clone())))
}

async fn v3_refresh(req: HttpRequest, session: web::Data<SessionService>) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let token = cookie(&req, SESSION_COOKIE);
    if token.is_empty() {
        let mut resp = HttpResponse::Unauthorized();
        attach_clear_session_cookie(&mut resp);
        return resp
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({
                "error": {
                    "code": "signIn",
                    "message": "กรุณาเข้าสู่ระบบ",
                }
            }));
    }
    if session.is_configured() {
        match branch_tablets(session.pool())
            .is_bound_session(&sha256_hex(&token))
            .await
        {
            Ok(true) => return refusal_response(401, "signIn", "กรุณาเข้าสู่ระบบ"),
            Ok(false) => {}
            Err(e) => {
                tracing::error!(error = %e, "branch tablet refresh check failed");
                return refusal_response(500, "serverError", "เซิร์ฟเวอร์ขัดข้อง");
            }
        }
    }
    let ip = client_address(&req);
    let now = Utc::now();
    match session.refresh(&token, Some(ip), now).await {
        Ok(user) => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({ "user": user })),
        Err(err) => {
            let (status, code, msg) = err.refusal();
            if let SessionError::Identity(IdentityError::Unavailable { path, status, code }) = &err
            {
                tracing::error!(
                    path = %path,
                    status = status,
                    code = %code,
                    "Identity unavailable on refresh"
                );
            }
            let mut resp =
                HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap());
            if status == 401 {
                attach_clear_session_cookie(&mut resp);
            }
            resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({
                    "error": {
                        "code": code,
                        "message": msg,
                    }
                }))
        }
    }
}

async fn v3_logout(req: HttpRequest, session: web::Data<SessionService>) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let token = cookie(&req, SESSION_COOKIE);
    let ip = client_address(&req);
    if !token.is_empty()
        && let Err(e) = session.logout(&token, Some(ip)).await
    {
        return handle_session_error(e);
    }
    let mut resp = HttpResponse::Ok();
    attach_logout_cookies(&mut resp);
    resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .json(json!({ "saved": true }))
}

fn validation_response(field: &str, message: &str) -> HttpResponse {
    HttpResponse::BadRequest()
        .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .json(json!({
            "error": {
                "code": "validationFailed",
                "field": field,
                "message": message,
            }
        }))
}

async fn v3_password(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง"),
    };
    let current_password = body
        .get("currentPassword")
        .and_then(Value::as_str)
        .unwrap_or("");
    if current_password.is_empty() {
        return validation_response("currentPassword", "กรุณากรอกรหัสผ่านปัจจุบัน");
    }
    let new_password = body
        .get("newPassword")
        .and_then(Value::as_str)
        .unwrap_or("");
    if new_password.is_empty() {
        return validation_response("newPassword", "กรุณากรอกรหัสผ่านใหม่");
    }
    let token = cookie(&req, SESSION_COOKIE);
    if token.is_empty() {
        return refusal_response(401, "signIn", "กรุณาเข้าสู่ระบบ");
    }
    let ip = client_address(&req);
    let now = Utc::now();
    match session
        .change_password(&token, current_password, new_password, Some(ip), now)
        .await
    {
        Ok(()) => {
            let mut resp = HttpResponse::Ok();
            attach_logout_cookies(&mut resp);
            resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({ "saved": true }))
        }
        Err(SessionError::Identity(IdentityError::InvalidCredentials)) => {
            refusal_response(401, "invalidCredentials", "ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง")
        }
        Err(SessionError::Identity(IdentityError::WeakPassword)) => {
            validation_response("newPassword", "รหัสผ่านไม่ผ่านเงื่อนไขความปลอดภัย")
        }
        Err(err) => handle_session_error(err),
    }
}
