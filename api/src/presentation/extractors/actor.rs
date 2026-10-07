//! Actor extractor: the session cookies, who they are, and the reader the pages use.

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_kernel::ids::BranchId;

use crate::presentation::pages::{SessionReader, Sessions};

pub type Accounts = AccountService<PgAccountRepository>;

pub const OWNER_COOKIE: &str = "__Host-xc_owner";
pub const STAFF_COOKIE: &str = "__Host-xc_staff";
/// v2's `OWNER_ID`: the id every owner acts under.
pub const OWNER_ID: &str = "owner";
pub const SESSION_SECONDS: u32 = 43_200;

pub const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

/// The value of the first cookie called `name`, or "".
pub fn cookie(req: &HttpRequest, name: &str) -> String {
    let all = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let prefix = format!("{name}=");
    all.split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix(prefix.as_str()))
        .unwrap_or("")
        .to_owned()
}

pub fn set_cookie(name: &str, value: &str, max_age: u32) -> (header::HeaderName, String) {
    (
        header::SET_COOKIE,
        format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}"),
    )
}

pub use super::origin::same_origin;

pub fn answer(status: u16, body: Value, no_store: bool) -> HttpResponse {
    let mut response = HttpResponse::build(StatusCode::from_u16(status).unwrap());
    if no_store {
        response.insert_header((header::CACHE_CONTROL, "no-store"));
    }
    response.json(body)
}

pub fn refuse(status: u16, sentence: &str) -> HttpResponse {
    answer(status, json!({ "error": sentence }), true)
}

pub fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError()
        .content_type("application/json; charset=utf-8")
        .body(json!({ "error": SERVER_ERROR }).to_string())
}

/// What a service's error becomes on these routes.
pub fn failure(error: AppError) -> HttpResponse {
    match error {
        AppError::Domain(sentence) => refuse(400, &sentence),
        AppError::Unauthenticated => refuse(401, "EMPLOYEE_AUTH_REQUIRED"),
        other => {
            tracing::error!(error = ?other, "request failed");
            server_error()
        }
    }
}

/// The body as JSON: `(action, payload)`. An empty body is `{}`; a missing payload is `{}`.
pub fn read(body: &web::Bytes, limit: usize, too_long: &str) -> Result<Value, HttpResponse> {
    let Ok(text) = std::str::from_utf8(body) else {
        return Err(refuse(400, BAD_REQUEST));
    };
    if text.encode_utf16().count() > limit {
        return Err(refuse(400, too_long));
    }
    if text.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(text).map_err(|_| refuse(400, BAD_REQUEST))
}

pub fn payload_of(body: &Value) -> Value {
    match body.get("payload") {
        Some(payload) if payload.is_object() => payload.clone(),
        _ => json!({}),
    }
}

pub fn accounts(req: &HttpRequest) -> Option<&web::Data<Accounts>> {
    req.app_data::<web::Data<Accounts>>()
}

pub fn staff_actor(e: &Employee) -> Option<Actor> {
    Actor::staff(
        e.id.clone(),
        Role::parse(&e.role)?,
        BranchId::new(e.branch_id.clone()),
    )
}

pub fn actor_of(who: &SignedIn) -> Option<Actor> {
    match who {
        SignedIn::Owner(_) => Some(Actor::owner(OWNER_ID)),
        SignedIn::Employee(e) => staff_actor(e),
    }
}

/// Who the two cookies are: the owner session first, else the staff session — the answer of
/// `AccountService::status`, except that a database failure is an error here.
///
/// stopgap: `AccountService::status` on `main` turns every error of `employee_session` into
/// "nobody" (`xc-auth/src/application/account/service.rs`, the `Err(_) => return Ok(None)` arm),
/// so with the database down a staff cookie would be answered 200 `user: null` instead of the
/// 500 of §8.1. The root cause is in `xc-auth` (issue #98); once `status` reports the failure
/// itself this function is one call to it.
pub async fn signed_in(
    accounts: &Accounts,
    owner_token: &str,
    staff_token: &str,
    now: SystemTime,
) -> AppResult<Option<SignedIn>> {
    if let Some(who) = accounts.status(owner_token, "", now.into()).await? {
        return Ok(Some(who));
    }
    if staff_token.is_empty() {
        return Ok(None);
    }
    match accounts
        .employee_session(staff_token, false, now.into())
        .await
    {
        Ok(employee) => Ok(Some(SignedIn::Employee(employee))),
        Err(AppError::Unauthenticated | AppError::Domain(_)) => Ok(None),
        Err(other) => Err(other),
    }
}

/// The reader `pages.rs` looks up as `web::Data<dyn SessionReader>`.
pub struct AccountSessionReader {
    pub accounts: Accounts,
}

impl SessionReader for AccountSessionReader {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            let now = SystemTime::now();
            let owner = match self
                .accounts
                .status(&cookie(req, OWNER_COOKIE), "", now.into())
                .await
            {
                Ok(Some(who @ SignedIn::Owner(_))) => actor_of(&who),
                _ => None,
            };
            let staff = match self
                .accounts
                .status("", &cookie(req, STAFF_COOKIE), now.into())
                .await
            {
                Ok(Some(who @ SignedIn::Employee(_))) => actor_of(&who),
                _ => None,
            };
            Sessions { owner, staff }
        })
    }
}

#[cfg(test)]
mod tests {
    use actix_web::body::to_bytes;

    use super::*;

    /// What `/api/auth` and `/api/staff/logout` answer when `AccountService::logout` reports
    /// that the database failed: v2's 500 sentence, never `{"saved":true}`.
    #[actix_web::test]
    async fn a_database_error_is_the_500_sentence_without_its_cause() {
        let response = failure(AppError::Database(sqlx::Error::PoolTimedOut));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("application/json"))
        );
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        let body = to_bytes(response.into_body()).await.expect("a body");
        assert_eq!(
            std::str::from_utf8(&body).expect("UTF-8"),
            r#"{"error":"เซิร์ฟเวอร์ขัดข้อง"}"#
        );
    }
}
