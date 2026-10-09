//! Auth routes: `/api/auth`, the staff door, and Identity BFF routes under `/api/auth/*`.

use std::sync::Arc;
use std::time::SystemTime;

use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use serde_json::{Value, json};
use xc_auth::application::account::service::AccountService;
use xc_auth::application::session::{SESSION_COOKIE, SessionError, SessionService};
use xc_auth::application::user::user_service::InviteRefusal;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::domain::identity::identity_client::{
    IdentityClient, IdentityError, UNAVAILABLE_MESSAGE,
};
use xc_auth::domain::token::TokenVerifier;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_auth::infrastructure::identity::HttpIdentityClient;
use xc_auth::infrastructure::token::http_jwks_source::HttpJwksSource;
use xc_auth::infrastructure::token::jwks_verifier::JwksTokenVerifier;

use crate::bootstrap::AppState;
use crate::presentation::extractors::actor::{
    AccountSessionReader, Accounts, BAD_REQUEST, OWNER_COOKIE, OWNER_ID, SESSION_SECONDS,
    STAFF_COOKIE, accounts, answer, cookie, failure, payload_of, read, refuse, same_origin,
    server_error, set_cookie,
};
use crate::presentation::http::client_address::{CLIENT_ADDRESS_HEADER, client_address};
use crate::presentation::pages::{self, SessionReader};

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let service: Accounts = AccountService::new(
        Arc::new(PgAccountRepository::new(state.db.clone())),
        PasswordHasher::new(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(AccountSessionReader {
        accounts: service.clone(),
    });
    cfg.app_data(web::Data::new(service));
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
        web::resource("/api/auth/invite/claim")
            .app_data(body())
            .route(web::post().to(v3_claim)),
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
        web::resource("/api/auth/register")
            .app_data(body())
            .route(web::post().to(v3_register)),
    );
    cfg.service(
        web::resource("/api/auth/session")
            .app_data(body())
            .route(web::post().to(v3_session)),
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

    // Old routes
    cfg.service(
        web::resource("/api/auth")
            .app_data(body())
            .route(web::post().to(auth))
            .default_service(web::to(pages::fallback)),
    );
    for (path, op) in [
        ("/api/staff/login", Op::Login),
        ("/api/staff/logout", Op::Logout),
        ("/api/staff/password", Op::Password),
        ("/api/staff/me", Op::Me),
    ] {
        cfg.service(
            web::resource(path)
                .app_data(body())
                .route(
                    web::post()
                        .to(move |req: HttpRequest, bytes: web::Bytes| staff(req, bytes, op)),
                )
                .default_service(web::to(pages::fallback)),
        );
    }
}

fn refusal_response(status: u16, code: &str, message: &str) -> HttpResponse {
    refusal_response_with_reason(status, code, message, None)
}

fn refusal_response_with_reason(
    status: u16,
    code: &str,
    message: &str,
    reason: Option<InviteRefusal>,
) -> HttpResponse {
    let mut err_obj = serde_json::Map::new();
    err_obj.insert("code".to_string(), json!(code));
    if let Some(reason) = reason {
        err_obj.insert("reason".to_string(), json!(reason));
    }
    err_obj.insert("message".to_string(), json!(message));
    HttpResponse::build(actix_web::http::StatusCode::from_u16(status).unwrap())
        .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .json(json!({
            "error": err_obj
        }))
}

fn handle_session_error(err: SessionError) -> HttpResponse {
    if let SessionError::Identity(IdentityError::Unavailable { path, status, code }) = &err {
        tracing::error!(
            path = %path,
            status = status,
            code = %code,
            "Identity unavailable"
        );
    }
    let reason = err.reason();
    let (status, code, msg) = err.refusal();
    refusal_response_with_reason(status, code, &msg, reason)
}

fn attach_signin_cookies(builder: &mut actix_web::HttpResponseBuilder, session_token: &str) {
    let session_cookie = format!(
        "__Host-xc_session={session_token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=43200"
    );
    let owner_cookie = "__Host-xc_owner=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    let staff_cookie = "__Host-xc_staff=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";

    builder.append_header((actix_web::http::header::SET_COOKIE, session_cookie));
    builder.append_header((actix_web::http::header::SET_COOKIE, owner_cookie));
    builder.append_header((actix_web::http::header::SET_COOKIE, staff_cookie));
}

fn attach_logout_cookies(builder: &mut actix_web::HttpResponseBuilder) {
    let session_cookie = "__Host-xc_session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    let owner_cookie = "__Host-xc_owner=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    let staff_cookie = "__Host-xc_staff=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";

    builder.append_header((actix_web::http::header::SET_COOKIE, session_cookie));
    builder.append_header((actix_web::http::header::SET_COOKIE, owner_cookie));
    builder.append_header((actix_web::http::header::SET_COOKIE, staff_cookie));
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

async fn v3_claim(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    if !session.is_configured() {
        tracing::error!(
            path = "/api/auth/invite/claim",
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
    let invite_code = body
        .get("inviteCode")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if identifier.is_empty() || password.is_empty() || invite_code.is_empty() {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let ip = client_address(&req);
    let now = Utc::now();
    match session
        .claim_invite(identifier, password, invite_code, Some(ip), now)
        .await
    {
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
    if destination.is_empty() || channel.is_empty() {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let ip = client_address(&req);
    match session.otp_request(destination, channel, Some(ip)).await {
        Ok(resp) => HttpResponse::Ok()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({
                "challengeId": resp.challenge_id,
                "expiresAt": resp.expires_at,
            })),
        Err(err) => handle_session_error(err),
    }
}

async fn v3_otp_verify(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
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
    let ip = client_address(&req);
    let now = Utc::now();
    match session.otp_verify(challenge_id, code, Some(ip), now).await {
        Ok(tail) => {
            let mut resp = HttpResponse::Ok();
            attach_signin_cookies(&mut resp, &tail.session_token);
            resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
                .json(json!({ "user": tail.user }))
        }
        Err(err) => handle_session_error(err),
    }
}

async fn v3_register(
    req: HttpRequest,
    bytes: web::Bytes,
    session: web::Data<SessionService>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    if !session.is_configured() {
        tracing::error!(
            path = "/api/auth/register",
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
    let phone = body.get("phone").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let email = body.get("email").and_then(|v| v.as_str());
    if phone.is_empty() || password.is_empty() {
        return refusal_response(400, "invalidRequest", "คำขอไม่ถูกต้อง");
    }
    let ip = client_address(&req);
    match session.register(phone, password, email, Some(ip)).await {
        Ok(_) => HttpResponse::Created()
            .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
            .json(json!({ "registered": true })),
        Err(err) => handle_session_error(err),
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
    if !token.is_empty() {
        let _ = session.logout(&token, Some(ip)).await;
    }
    let mut resp = HttpResponse::Ok();
    attach_logout_cookies(&mut resp);
    resp.insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .json(json!({ "saved": true }))
}

/// v2's `safe(employee)`.
pub fn employee_json(e: &Employee) -> Value {
    json!({ "id": e.id, "username": e.username, "name": e.name, "role": e.role,
            "branchId": e.branch_id, "active": e.active, "mustChange": e.must_change })
}

/// v2's `publicUser`.
fn user_json(who: &SignedIn) -> Value {
    match who {
        SignedIn::Owner(o) => {
            json!({ "id": OWNER_ID, "username": o.username, "name": o.name, "role": "OWNER", "branchId": null })
        }
        SignedIn::Employee(e) => {
            json!({ "id": e.id, "username": e.username, "name": e.name, "role": e.role, "branchId": e.branch_id })
        }
    }
}

async fn auth(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST);
    }
    let body = match read(&bytes, 4000, "ข้อมูลคำขอใหญ่เกินกำหนด")
    {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };
    let payload = payload_of(&body);
    let ip = client_address(&req);
    let (owner, staff) = (cookie(&req, OWNER_COOKIE), cookie(&req, STAFF_COOKIE));
    let now = SystemTime::now().into();
    match body["action"].as_str() {
        Some("status") => match accounts.status(&owner, &staff, now).await {
            Ok(who) => answer(
                200,
                json!({ "setupRequired": false, "user": who.as_ref().map(user_json) }),
                true,
            ),
            Err(e) => failure(e),
        },
        Some("login") => match accounts.login(&payload, ip, now).await {
            Ok(login) => {
                let mut body = json!({ "user": user_json(&login.who) });
                let (set, clear) = match &login.who {
                    SignedIn::Owner(_) => (OWNER_COOKIE, STAFF_COOKIE),
                    SignedIn::Employee(e) => {
                        body["mustChange"] = json!(e.must_change);
                        (STAFF_COOKIE, OWNER_COOKIE)
                    }
                };
                let mut response = answer(200, body, true);
                append(
                    &mut response,
                    set_cookie(set, &login.token, SESSION_SECONDS),
                );
                append(&mut response, set_cookie(clear, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        Some("logout") => match accounts.logout(&owner, &staff).await {
            Ok(()) => {
                let mut response = answer(200, json!({ "saved": true }), true);
                append(&mut response, set_cookie(OWNER_COOKIE, "", 0));
                append(&mut response, set_cookie(STAFF_COOKIE, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        _ => refuse(400, BAD_REQUEST),
    }
}

fn append(
    response: &mut HttpResponse,
    (name, value): (actix_web::http::header::HeaderName, String),
) {
    response
        .headers_mut()
        .append(name, value.parse().expect("an ASCII cookie"));
}

#[derive(Clone, Copy)]
enum Op {
    Login,
    Logout,
    Password,
    Me,
}

async fn staff(req: HttpRequest, bytes: web::Bytes, op: Op) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST);
    }
    let body = match read(&bytes, 120_000, "ข้อมูลมากเกินไป") {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };
    let payload = payload_of(&body);
    let token = cookie(&req, STAFF_COOKIE);
    let now = SystemTime::now().into();
    match op {
        Op::Login => {
            let ip = client_address(&req);
            match accounts.login_employee(&payload, ip, now).await {
                Ok(login) => {
                    let SignedIn::Employee(e) = &login.who else {
                        return server_error();
                    };
                    let mut response = answer(200, json!({ "employee": employee_json(e) }), true);
                    append(
                        &mut response,
                        set_cookie(STAFF_COOKIE, &login.token, SESSION_SECONDS),
                    );
                    response
                }
                Err(e) => failure(e),
            }
        }
        Op::Logout => match accounts.logout("", &token).await {
            Ok(()) => {
                let mut response = answer(200, json!({ "saved": true }), true);
                append(&mut response, set_cookie(STAFF_COOKIE, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        Op::Password => match accounts.change_password(&token, &payload, now).await {
            Ok(()) => {
                let mut response = answer(200, json!({ "saved": true, "loginAgain": true }), true);
                append(&mut response, set_cookie(STAFF_COOKIE, "", 0));
                response
            }
            Err(e) => failure(e),
        },
        Op::Me => match accounts.employee_session(&token, true, now).await {
            Ok(e) => answer(200, json!({ "employee": employee_json(&e) }), true),
            Err(e) => failure(e),
        },
    }
}
