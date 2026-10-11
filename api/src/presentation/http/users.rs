//! `userCreate` (#1157): `POST /api/users`. The shop owner (or an ADMIN) creates a staff member and
//! the person's Identity account server-side; the person then signs in on `/login` with the password
//! the owner set. Nobody signs up from outside.
//!
//! This route is not a `CommandAction`: the ticket's order (design §5.3) commits the `PENDING` row
//! before Identity is called and binds in a new transaction, so it cannot run inside the command
//! endpoint's one transaction. That is why it lives here and not under `v3/` (lane controller's ruling
//! on #1157, option A). It uses the same sign-in and `Policy::authorize` gate, on the
//! `(Rpc, "userCreate")` row (`xc-auth` `areas/user_create.rs`, `UsersManage`).

use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use serde::{Deserialize, Serialize};
use xc_auth::application::user::UserService;
use xc_auth::application::user::user_service::{CreateStaffError, StaffCreate};
use xc_auth::domain::access::{Role, ScopedRole};
use xc_auth::domain::identity::identity_client::{IdentityClient, IdentityError};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_auth::infrastructure::identity::http_identity_client::HttpIdentityClient;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::client_address::CLIENT_ADDRESS_HEADER;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::authenticate_and_authorize;

const INHERITS: (EntryPath, &str) = (EntryPath::Rpc, "userCreate");

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCreateRole {
    pub role: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCreateInput {
    pub name: String,
    pub username: Option<String>,
    /// Required; `validationFailed` on `phone` when missing.
    pub phone: Option<String>,
    pub email: Option<String>,
    /// Sent to Identity only; never stored or echoed by xchange.
    pub password: String,
    pub roles: Vec<UserCreateRole>,
    /// Completes a `PENDING` user left by an earlier attempt (Identity refused or was down).
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCreateOutput {
    pub user_id: String,
    pub status: String,
    pub version: i32,
}

fn invalid(field: &'static str, message: impl Into<String>) -> HttpResponse {
    V3Error::new(
        "validationFailed",
        Some(field),
        message,
        StatusCode::BAD_REQUEST,
    )
    .error_response()
}

async fn user_create(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Bytes,
) -> HttpResponse {
    // A write from another site is refused before anything else, as on every other write route
    // (`v3/command.rs` step 2, `auth.rs`, `employees.rs`).
    if !same_origin(&req) {
        return V3Error::new(
            "originRejected",
            None,
            "ORIGIN_REJECTED",
            StatusCode::BAD_REQUEST,
        )
        .error_response();
    }

    let actor = match authenticate_and_authorize(&req, INHERITS).await {
        Ok(callers) => callers,
        Err(response) => return response,
    };

    let input: UserCreateInput = match serde_json::from_slice(&body) {
        Ok(input) => input,
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
    let phone = match input.phone.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => return invalid("phone", "กรุณากรอกเบอร์โทรศัพท์"),
    };

    let now = chrono::Utc::now();
    let mut roles = Vec::with_capacity(input.roles.len());
    for r in &input.roles {
        let Some(role) = Role::parse(&r.role) else {
            return invalid("roles", format!("ไม่รู้จักบทบาท {}", r.role));
        };
        let branch = match r.branch.as_deref() {
            None | Some("") => None,
            Some(b) if BranchId::is_well_formed(b) => Some(BranchId::new(b)),
            Some(_) => return invalid("roles", "สาขาไม่ถูกต้อง"),
        };
        roles.push(ScopedRole::new(role, branch, now, None));
    }

    let Some(identity_settings) = state.settings.identity.clone() else {
        let (status, code, message) =
            IdentityError::unavailable("/auth/register", 503, "not_configured").refusal();
        return V3Error::new(
            code,
            None,
            message,
            StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
        )
        .error_response();
    };
    let identity: Arc<dyn IdentityClient> = Arc::new(HttpIdentityClient::new(
        identity_settings,
        CLIENT_ADDRESS_HEADER,
    ));

    let users = UserService::new(Arc::new(PgUserRepository::new()));
    let create = StaffCreate {
        name: &input.name,
        username: input.username.as_deref().filter(|u| !u.trim().is_empty()),
        phone: &phone,
        email: input.email.as_deref().filter(|e| !e.trim().is_empty()),
        password: &input.password,
        roles,
        must_change_password: false,
        retry_user_id: input.user_id.as_deref(),
        created_by: &actor.user_id,
        creator_roles: Some(&actor.roles),
    };

    match users
        .create_staff(
            &state.db,
            identity.as_ref(),
            &actor.organization_id,
            &create,
            now,
        )
        .await
    {
        Ok(user) => HttpResponse::Ok().json(UserCreateOutput {
            user_id: user.id,
            status: user.status.as_str().to_string(),
            version: user.version,
        }),
        Err(CreateStaffError::Validation { field, message }) => invalid(field, message),
        Err(CreateStaffError::OutOfScope) => V3Error::new(
            "outOfScope",
            Some("roles"),
            PolicyError::OutOfScope.to_string(),
            StatusCode::FORBIDDEN,
        )
        .error_response(),
        Err(CreateStaffError::Identity(e)) => {
            let (status, code, message) = e.refusal();
            let field = match e {
                IdentityError::PhoneRequired | IdentityError::InvalidPhone => Some("phone"),
                _ => None,
            };
            let (code, status) = match e {
                // §7: a missing or wrong phone is `validationFailed` on `phone`.
                IdentityError::PhoneRequired | IdentityError::InvalidPhone => {
                    ("validationFailed", StatusCode::BAD_REQUEST)
                }
                _ => (
                    code,
                    StatusCode::from_u16(status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
                ),
            };
            V3Error::new(code, field, message, status).error_response()
        }
        Err(CreateStaffError::BindFailed {
            user_id,
            identity_id,
        }) => V3Error::new(
            "bindFailed",
            None,
            format!(
                "สร้างบัญชีที่ระบบยืนยันตัวตนแล้ว ({identity_id}) แต่ผูกกับผู้ใช้ {user_id} ไม่สำเร็จ บันทึกไว้แล้ว"
            ),
            StatusCode::CONFLICT,
        )
        .error_response(),
        Err(CreateStaffError::App(e)) => {
            tracing::error!(error = %e, "userCreate failed");
            V3Error::new(
                "serverError",
                None,
                "เซิร์ฟเวอร์ขัดข้อง",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response()
        }
    }
}

pub fn configure(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.service(web::resource("/api/users").route(web::post().to(user_create)));
}
