//! Actor extractor: the session cookies, who they are, and the reader the pages use.

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use pos_error::AppError;
use serde_json::{Value, json};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::access::Grant;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_kernel::ids::BranchId;

use crate::presentation::pages::{Callers, SessionReader, Sessions};

pub type Accounts = AccountService<PgAccountRepository>;

pub const SESSION_COOKIE: &str = "__Host-xc_session";
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
            tracing::error!(error = %other, "request failed");
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

use crate::presentation::extractors::session_actor::{actor_of_user, resolve_session_actor};
use chrono::{DateTime, Utc};
use xc_auth::application::session::SessionService;
use xc_auth::domain::access::{ScopedRole, Standing};

/// The reader `pages.rs` looks up as `web::Data<dyn SessionReader>`.
pub struct AccountSessionReader {
    pub accounts: Accounts,
    pub org_id: String,
}

impl SessionReader for AccountSessionReader {
    #[allow(clippy::collapsible_if)]
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let session_token = cookie(req, SESSION_COOKIE);
            if !session_token.is_empty() {
                if let Some(session_svc) = req.app_data::<web::Data<SessionService>>() {
                    let now_utc = Utc::now();
                    // SB-1 §14.3: the actor works in the session's effective branch. Reading never writes it.
                    // #654: its `customer_id` comes from the customer link (`resolve_session_actor`).
                    if let Some(access_actor) = resolve_session_actor(
                        session_svc,
                        session_svc.pool(),
                        &session_token,
                        now_utc,
                    )
                    .await
                    {
                        let staff_user = Standing::of(&access_actor, now_utc)
                            .filter(|s| !s.is_owner())
                            .map(|_| access_actor.clone());
                        return Callers {
                            user: Some(access_actor),
                            staff_user,
                        };
                    }
                }
            }
            let s = self.sessions(req).await;
            crate::presentation::pages::old_cookie_callers(s, &self.org_id)
        })
    }

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

/// The password session a request carries: the owner cookie's first, else the staff cookie's.
/// Routes read v2 sessions only through this and [`staff_password_session`] (OR-4b #767), so no
/// route names a cookie.
pub async fn password_session(
    accounts: &Accounts,
    req: &HttpRequest,
    now: DateTime<Utc>,
) -> pos_error::AppResult<Option<SignedIn>> {
    accounts
        .status(&cookie(req, OWNER_COOKIE), &cookie(req, STAFF_COOKIE), now)
        .await
}

/// The staff password session alone (an owner cookie counts for nothing), as
/// `AccountService::employee_session` answers it.
pub async fn staff_password_session(
    accounts: &Accounts,
    req: &HttpRequest,
    allow_must_change: bool,
    now: DateTime<Utc>,
) -> pos_error::AppResult<Employee> {
    accounts
        .employee_session(&cookie(req, STAFF_COOKIE), allow_must_change, now)
        .await
}

/// The Blueprint session (`__Host-xc_session`) a request carries: its user and roles, or `None`
/// when there is no such cookie or it is not an active session.
pub async fn identity_session(
    req: &HttpRequest,
    now: DateTime<Utc>,
) -> Option<(xc_auth::domain::user::User, Vec<ScopedRole>)> {
    let token = cookie(req, SESSION_COOKIE);
    if token.is_empty() {
        return None;
    }
    let sessions = req.app_data::<web::Data<SessionService>>()?;
    sessions
        .active_user_and_roles(&token, now)
        .await
        .ok()
        .flatten()
}

/// The `sha256_hex` of the Blueprint session (`__Host-xc_session`) a request carries when that
/// hash is a live `user_sessions` row (`expires > now`), or `None` otherwise (#1943, §9.2).
pub async fn caller_session_hash(req: &HttpRequest, now: DateTime<Utc>) -> Option<String> {
    let token = cookie(req, SESSION_COOKIE);
    if token.is_empty() {
        return None;
    }
    let pool = req
        .app_data::<web::Data<crate::bootstrap::AppState>>()
        .map(|s| &s.db)
        .or_else(|| {
            req.app_data::<web::Data<SessionService>>()
                .map(|s| s.pool())
        })?;
    let hash = xc_auth::application::session::sha256_hex(&token);
    sqlx::query_scalar::<_, String>(
        "SELECT hash FROM user_sessions WHERE hash = $1 AND expires > $2",
    )
    .bind(&hash)
    .bind(now)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// The Blueprint session a request carries as its user and `access::Actor` (`customer_id` from the customer link,
/// #654), or `None` as [`identity_session`] answers it.
pub async fn identity_session_actor(
    req: &HttpRequest,
    now: DateTime<Utc>,
) -> Option<(xc_auth::domain::user::User, xc_auth::domain::access::Actor)> {
    let (user, roles) = identity_session(req, now).await?;
    let sessions = req.app_data::<web::Data<SessionService>>()?;
    let actor = actor_of_user(sessions.pool(), &user, roles).await?;
    Some((user, actor))
}

pub async fn callers(req: &HttpRequest) -> Callers {
    if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        reader.callers(req).await
    } else {
        Callers {
            user: None,
            staff_user: None,
        }
    }
}

pub fn is_staff_path(path: EntryPath) -> bool {
    matches!(
        path,
        EntryPath::StaffRpc | EntryPath::StaffDesk | EntryPath::StaffFile | EntryPath::StaffUpload
    )
}

pub async fn authorize(
    req: &HttpRequest,
    path: EntryPath,
    action: &Action,
    named: Option<&BranchId>,
) -> Result<(Grant, Callers), HttpResponse> {
    let callers = callers(req).await;
    let actor = if is_staff_path(path) {
        callers.staff_user.as_ref()
    } else {
        callers.user.as_ref()
    };
    match Policy::authorize(actor, path, action, named, Utc::now()) {
        Ok(grant) => Ok((grant, callers)),
        Err(e) => Err(refuse(e.status(), &e.to_string())),
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
