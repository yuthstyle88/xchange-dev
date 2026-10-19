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

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
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
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input);
        todo!("F12")
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
