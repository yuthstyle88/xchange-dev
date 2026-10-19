//! RBAC F12 (design `docs/design/blueprint/user-management-rbac.md` §11.3, §11.6): add, edit and delete a group.
//!
//! - `adminGroupAdd` (`POST /api/admin/groups/add`): a custom group with a name, a base and a set.
//! - `adminGroupSave` (`POST /api/admin/groups/save`): replace a group's set.
//! - `adminGroupDelete` (`POST /api/admin/groups/delete`): delete a custom group nobody holds.
//!
//! Each needs `UsersManage` on its policy row (`org_admin.rs`) **and** a valid organisation-wide row whose base is
//! EXECUTIVE (§11.3, V.9 :66), else `403 roleNotAllowed`. A save or add runs `MatrixGuardrails::check` on the full
//! set (requested module permissions plus the base's no-module seed) and answers `409 guardrail:<n>`. Each writes one
//! `user_audit` row (`PERMISSION_CHANGE` / `GROUP_ADD` / `GROUP_DELETE`, subject = group code, before and after) and
//! installs the new snapshot after commit.
//!
//! Skeleton (claude/65357afb, `feat/rbac-groups`): the `run` bodies are F12's.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::application::user::account_auditor::AccountAuditor;
use xc_auth::domain::access::permission_module::PermissionModule;
use xc_auth::domain::access::{
    Actor, Group, GuardrailBreach, MatrixGuardrails, Permission, Role, RoleGrantStore,
    RoleGrantStoreError, RoleGrants,
};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

use super::user_grant::validation_failed;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, request_ip, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_groups.ts")]
pub struct AdminGroupAddInput {
    /// The new group's code: `A-Z`, `0-9` and `_`, 2 to 40 characters, not a built-in code.
    pub code: String,
    /// Shown on screen: 1 to 80 characters after trimming.
    pub name: String,
    /// A built-in role code other than `EXECUTIVE` and `CUSTOMER`.
    pub base: String,
    /// Permission serde names; module permissions only (a no-module one is `409 guardrail:5`).
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_groups.ts")]
pub struct AdminGroupSaveInput {
    pub code: String,
    /// The group's module permissions after the save; its no-module ones are kept from its base's seed.
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_groups.ts")]
pub struct AdminGroupDeleteInput {
    pub code: String,
}

/// The group changed and `role_grant_version` after the change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_groups.ts")]
pub struct AdminGroupOutput {
    pub code: String,
    #[ts(type = "number")]
    pub version: i64,
}

pub struct AdminGroupAddCommand(#[allow(dead_code)] PgPool);
pub struct AdminGroupSaveCommand(#[allow(dead_code)] PgPool);
pub struct AdminGroupDeleteCommand(#[allow(dead_code)] PgPool);

impl CommandAction for AdminGroupAddCommand {
    const NAME: &'static str = "adminGroupAdd";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminGroupAdd");
    type Input = AdminGroupAddInput;
    type Output = AdminGroupOutput;
    const ROUTE: &'static str = "/api/admin/groups/add";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        require_executive(who)?;
        validate_code(&input.code)?;
        let name = validate_name(&input.name)?;
        let base = parse_base(&input.base)?;
        let requested = parse_permissions(&input.permissions)?;
        let group = Group {
            code: input.code.clone(),
            name,
            base,
            built_in: false,
        };
        let full = check_and_build_full_set(&group, &requested)?;
        PgRoleGrantStore::new()
            .add_group(tx, &group, &full)
            .await
            .map_err(from_store_error)?;
        let full_set: BTreeSet<Permission> = full.into_iter().collect();
        let after = group_audit_json(&group, &full_set);
        record_audit_and_reload(tx, who, input.code, "GROUP_ADD", None, Some(after)).await
    }
}

impl CommandAction for AdminGroupSaveCommand {
    const NAME: &'static str = "adminGroupSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminGroupSave");
    type Input = AdminGroupSaveInput;
    type Output = AdminGroupOutput;
    const ROUTE: &'static str = "/api/admin/groups/save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input);
        todo!("F12")
    }
}

impl CommandAction for AdminGroupDeleteCommand {
    const NAME: &'static str = "adminGroupDelete";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminGroupDelete");
    type Input = AdminGroupDeleteInput;
    type Output = AdminGroupOutput;
    const ROUTE: &'static str = "/api/admin/groups/delete";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input);
        todo!("F12")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(AdminGroupAddCommand(state.db.clone())));
    CommandEndpoint::register(cfg, Arc::new(AdminGroupSaveCommand(state.db.clone())));
    CommandEndpoint::register(cfg, Arc::new(AdminGroupDeleteCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminGroupAddInput::export_all(&cfg)?;
    AdminGroupSaveInput::export_all(&cfg)?;
    AdminGroupDeleteInput::export_all(&cfg)?;
    AdminGroupOutput::export_all(&cfg)?;
    Ok(())
}

// ------------------------------------------------------------ shared helpers

#[allow(dead_code)]
fn require_executive(who: &Actor) -> Result<(), V3Error> {
    let now = Utc::now();
    let is_exec = who.roles.iter().any(|r| {
        r.role == Role::Executive
            && r.branch.is_none()
            && r.valid_from <= now
            && r.valid_to.is_none_or(|to| now < to)
    });
    if is_exec {
        Ok(())
    } else {
        Err(V3Error::new(
            "roleNotAllowed",
            None,
            "role not allowed",
            StatusCode::FORBIDDEN,
        ))
    }
}

#[allow(dead_code)]
fn validate_code(code: &str) -> Result<(), V3Error> {
    let ok = (2..=40).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(validation_failed(Some("code"), "invalid group code"))
    }
}

#[allow(dead_code)]
fn validate_name(name: &str) -> Result<String, V3Error> {
    let trimmed = name.trim();
    if (1..=80).contains(&trimmed.chars().count()) {
        Ok(trimmed.to_owned())
    } else {
        Err(validation_failed(Some("name"), "invalid group name"))
    }
}

#[allow(dead_code)]
fn parse_base(base: &str) -> Result<Role, V3Error> {
    match Role::parse(base) {
        Some(role) if !matches!(role, Role::Executive | Role::Customer) => Ok(role),
        _ => Err(validation_failed(Some("base"), "invalid base role")),
    }
}

#[allow(dead_code)]
fn parse_permissions(names: &[String]) -> Result<Vec<Permission>, V3Error> {
    names
        .iter()
        .map(|name| {
            serde_json::from_value::<Permission>(Value::String(name.clone()))
                .map_err(|_| validation_failed(Some("permissions"), "unknown permission"))
        })
        .collect()
}

#[allow(dead_code)]
fn guardrail_error(breach: GuardrailBreach) -> V3Error {
    let code = format!("guardrail:{}", breach.number());
    V3Error::new(code.clone(), None, code, StatusCode::CONFLICT)
}

#[allow(dead_code)]
fn check_and_build_full_set(
    group: &Group,
    requested: &[Permission],
) -> Result<Vec<Permission>, V3Error> {
    let req_set: BTreeSet<Permission> = requested.iter().copied().collect();
    let seed_no_module: BTreeSet<Permission> = RoleGrants::seed(group.base)
        .into_iter()
        .filter(|p| PermissionModule::module(*p).is_none())
        .collect();
    let full: Vec<Permission> = Permission::ALL
        .into_iter()
        .filter(|p| {
            if PermissionModule::module(*p).is_some() {
                req_set.contains(p)
            } else {
                seed_no_module.contains(p)
            }
        })
        .collect();
    MatrixGuardrails::check(group, &full).map_err(guardrail_error)?;
    if requested
        .iter()
        .any(|p| PermissionModule::module(*p).is_none())
    {
        return Err(guardrail_error(GuardrailBreach::NoModuleEdited));
    }
    Ok(full)
}

#[allow(dead_code)]
fn not_found() -> V3Error {
    V3Error::new("notFound", None, "group not found", StatusCode::NOT_FOUND)
}

#[allow(dead_code)]
fn from_store_error(err: RoleGrantStoreError) -> V3Error {
    match err {
        RoleGrantStoreError::NotFound(_) => not_found(),
        RoleGrantStoreError::CodeTaken(msg) => {
            V3Error::new("codeTaken", None, msg, StatusCode::CONFLICT)
        }
        RoleGrantStoreError::BuiltIn(msg) => {
            V3Error::new("builtIn", None, msg, StatusCode::CONFLICT)
        }
        RoleGrantStoreError::InUse(msg) => V3Error::new("inUse", None, msg, StatusCode::CONFLICT),
        RoleGrantStoreError::Database(e) => {
            tracing::error!(error = %e, "role grant store failed");
            server_error()
        }
    }
}

#[allow(dead_code)]
fn group_audit_json(group: &Group, set: &BTreeSet<Permission>) -> Value {
    let permissions: Vec<Permission> = Permission::ALL
        .into_iter()
        .filter(|p| set.contains(p))
        .collect();
    json!({
        "code": group.code,
        "name": group.name,
        "base": group.base.as_str(),
        "permissions": permissions,
    })
}

#[allow(dead_code)]
async fn record_audit_and_reload(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    who: &Actor,
    code: String,
    action: &str,
    before: Option<Value>,
    after: Option<Value>,
) -> Result<AdminGroupOutput, V3Error> {
    AccountAuditor::record(
        &PgUserRepository::new(),
        tx,
        &who.user_id,
        &code,
        action,
        &who.organization_id,
        before,
        after,
        request_ip().as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "group audit failed");
        server_error()
    })?;
    let snapshot = PgRoleGrantStore::new()
        .load(&mut **tx)
        .await
        .map_err(from_store_error)?;
    let version = snapshot.version();
    RoleGrants::install(snapshot);
    Ok(AdminGroupOutput { code, version })
}

