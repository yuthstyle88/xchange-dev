//! OR-5a (#945): `POST /api/users/grant` (`userGrant`). A05: grant a scoped role (`UserService::grant`); `users.version` moves.
//!
//! The organisation check, the error mapping and the `{userId, status, version}` answer that
//! `userRevoke` and `userDisable` share live here, beside the shared `UserChangeOutput`: `build.rs`
//! registers every top-level file of `commands/` as a command, so they cannot have a file of their own.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::application::user::user_service::{UserService, creator_may_grant};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_grant.ts")]
pub struct UserGrantInput {
    pub user_id: String,
    pub role: String,
    /// The role's branch, `null` for organisation-wide. The command seam also authorises it.
    pub branch: Option<String>,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
}

/// The user after the change: its id, status and the new `users.version`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "user_grant.ts")]
pub struct UserChangeOutput {
    pub user_id: String,
    pub status: String,
    pub version: i32,
}

pub struct UserGrantCommand(#[allow(dead_code)] PgPool);

impl CommandAction for UserGrantCommand {
    const NAME: &'static str = "userGrant";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "userGrant");
    type Input = UserGrantInput;
    type Output = UserChangeOutput;
    const ROUTE: &'static str = "/api/users/grant";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let role = parse_role(&input.role)?;
        let valid_from = match input.valid_from.as_deref() {
            Some(text) => parse_instant(text, "validFrom")?,
            None => Utc::now(),
        };
        let valid_to = match input.valid_to.as_deref() {
            Some(text) => Some(parse_instant(text, "validTo")?),
            None => None,
        };
        if valid_to.is_some_and(|to| to <= valid_from) {
            return Err(validation_failed(
                Some("validTo"),
                "validTo must be after validFrom",
            ));
        }
        let role_branch = role_branch(input.branch.as_deref())?;
        if !creator_may_grant(
            &who.roles,
            &[ScopedRole::new(
                role,
                role_branch.clone(),
                valid_from,
                valid_to,
            )],
            Utc::now(),
        ) {
            return Err(out_of_scope());
        }
        let user_id = user_in_organisation(tx, branch, &input.user_id).await?;
        users()
            .grant(tx, who, None, &user_id, role, role_branch, valid_from, valid_to)
            .await
            .map_err(from_change_error)?;
        changed(tx, &user_id).await
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(UserGrantCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UserGrantInput::export_all(&cfg)?;
    UserChangeOutput::export_all(&cfg)?;
    Ok(())
}

// ------------------------------------------------------------ shared by userGrant, userRevoke, userDisable

pub(crate) fn users() -> UserService {
    UserService::new(Arc::new(PgUserRepository::new()))
}

pub(crate) fn validation_failed(field: Option<&'static str>, message: impl Into<String>) -> V3Error {
    V3Error::new("validationFailed", field, message, StatusCode::BAD_REQUEST)
}

/// A Blueprint role name (`role.rs`), else `validationFailed` on `role`.
pub(crate) fn parse_role(text: &str) -> Result<Role, V3Error> {
    Role::parse(text).ok_or_else(|| validation_failed(Some("role"), "unknown role"))
}

/// The role's own branch: `null` is organisation-wide.
pub(crate) fn role_branch(text: Option<&str>) -> Result<Option<BranchId>, V3Error> {
    match text {
        None => Ok(None),
        Some(id) if id.trim().is_empty() => {
            Err(validation_failed(Some("branch"), "branch is empty"))
        }
        Some(id) => Ok(Some(BranchId::new(id))),
    }
}

fn parse_instant(text: &str, field: &'static str) -> Result<DateTime<Utc>, V3Error> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| validation_failed(Some(field), "not an RFC 3339 instant"))
}

/// The user `user_id`, locked for this transaction, when it belongs to the organisation of the
/// branch the seam authorised; a user of another organisation is `outOfScope`, an unknown one
/// `notFound`. Answers the user's id as stored.
pub(crate) async fn user_in_organisation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    user_id: &str,
) -> Result<String, V3Error> {
    let row: Option<(String, Option<bool>)> = sqlx::query_as(
        "SELECT u.id::text, u.organization_id = b.organization_id \
         FROM users u LEFT JOIN branches b ON b.id = $2 \
         WHERE u.id::text = $1 FOR UPDATE OF u",
    )
    .bind(user_id)
    .bind(branch.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "failed to read the user");
        server_error()
    })?;
    let Some((id, same_organisation)) = row else {
        return Err(V3Error::new(
            "notFound",
            Some("userId"),
            "no such user",
            StatusCode::NOT_FOUND,
        ));
    };
    if same_organisation != Some(true) {
        return Err(out_of_scope());
    }
    Ok(id)
}

/// The cash guard used by account commands (design §3.2; C3 writes the query).
pub(crate) fn custody_holds() -> std::sync::Arc<dyn xc_auth::domain::user::CustodyHolds> {
    std::sync::Arc::new(xc_cash::infrastructure::custody_holds::PgCustodyHolds)
}

/// Maps an account-change refusal to the wire (design §3.6; codes in the C1a ticket §7, plus `409 statusNotAllowed`,
/// ruling #564 issuecomment-6246076019 Q6). Skeleton: only `App` is mapped; the rest is the RBAC function ticket.
pub(crate) fn from_change_error(e: xc_auth::application::user::user_service::UserChangeError) -> V3Error {
    use xc_auth::application::user::user_service::UserChangeError;
    use xc_auth::domain::access::GrantRefusal;

    match e {
        UserChangeError::NotFound => {
            V3Error::new("notFound", None, "user not found", StatusCode::NOT_FOUND)
        }
        UserChangeError::Refused(GrantRefusal::NoScope) => out_of_scope(),
        UserChangeError::Refused(GrantRefusal::SelfChange) => {
            V3Error::new("selfChange", None, "self change", StatusCode::FORBIDDEN)
        }
        UserChangeError::Refused(
            GrantRefusal::ProtectedTarget
            | GrantRefusal::ProtectedRole
            | GrantRefusal::EnableNeedsExecutive,
        ) => V3Error::new("roleNotAllowed", None, "role not allowed", StatusCode::FORBIDDEN),
        UserChangeError::Refused(GrantRefusal::LastOwner) => {
            V3Error::new("lastOwner", None, "last owner", StatusCode::CONFLICT)
        }
        UserChangeError::Refused(GrantRefusal::Scope(e)) => {
            validation_failed(None, format!("Scope rule rejected: {e}"))
        }
        UserChangeError::NotActive => {
            V3Error::new("userNotActive", None, "user not active", StatusCode::CONFLICT)
        }
        UserChangeError::HoldsCash => {
            V3Error::new(
                "holdsCash",
                None,
                "user holds cash or a pending handover",
                StatusCode::CONFLICT,
            )
        }
        UserChangeError::NotBound => {
            V3Error::new(
                "notBound",
                None,
                "user not bound to an identity",
                StatusCode::CONFLICT,
            )
        }
        UserChangeError::BadMove { from, to } => V3Error::new(
            "statusNotAllowed",
            None,
            format!("status move {from} -> {to} not allowed"),
            StatusCode::CONFLICT,
        ),
        UserChangeError::App(e) => from_app_error(e),
    }
}

/// A refusal of `UserService` (OR-2's scope rule, a disabled user) is `validationFailed`; a
/// database failure is `serverError`.
pub(crate) fn from_app_error(e: AppError) -> V3Error {
    match e {
        AppError::Domain(message) | AppError::Validation(message) => {
            validation_failed(None, message)
        }
        AppError::Conflict(message) => V3Error::new(
            "conflict",
            None,
            message,
            StatusCode::CONFLICT,
        ),
        other => {
            tracing::error!(error = %other, "user command failed");
            server_error()
        }
    }
}

/// The answer of every user command: the user's id, status and `users.version` after the change.
pub(crate) async fn changed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
) -> Result<UserChangeOutput, V3Error> {
    let (status, version): (String, i32) =
        sqlx::query_as("SELECT status, version FROM users WHERE id = $1::uuid")
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "failed to read the changed user");
                server_error()
            })?;
    Ok(UserChangeOutput {
        user_id: user_id.to_owned(),
        status,
        version,
    })
}
