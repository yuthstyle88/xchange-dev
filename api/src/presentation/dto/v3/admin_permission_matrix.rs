//! RBAC F5 (design `docs/design/blueprint/user-management-rbac.md` §11.6): the `admin_permission_matrix` view's data.
//! Every staff group (CUSTOMER is not shown), its base, and every permission with its module and level.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_permission_matrix.ts")]
pub struct AdminPermissionMatrixData {
    /// `role_grant_version`.
    #[ts(type = "number")]
    pub version: i64,
    /// The ten V.9 modules, in V.9's column order (`Module::as_str`).
    pub modules: Vec<String>,
    /// Every permission, in `Permission::ALL` order.
    pub permissions: Vec<MatrixPermission>,
    /// Built-in groups first in `RoleGrants::BUILT_IN` order (CUSTOMER left out), then custom groups by code.
    pub groups: Vec<MatrixGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_permission_matrix.ts")]
pub struct MatrixPermission {
    /// The serde name, e.g. `ExchangeTransact`.
    pub name: String,
    /// `Module::as_str`, `null` for a no-module permission.
    pub module: Option<String>,
    /// `Level::as_str`, `null` for a no-module permission.
    pub level: Option<String>,
    /// False for a no-module permission (§11.3 rule 5).
    pub editable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_permission_matrix.ts")]
pub struct MatrixGroup {
    pub code: String,
    pub name: String,
    pub base: String,
    pub built_in: bool,
    /// False for the built-in EXECUTIVE group (§11.3 rule 4), and for every group when the caller is not EXECUTIVE.
    pub editable: bool,
    /// The group's permissions, serde names, in `Permission::ALL` order.
    pub permissions: Vec<String>,
    /// The highest level per module among them (`PermissionMatrix::of`), keyed by `Module::as_str`.
    pub levels: std::collections::BTreeMap<String, String>,
}
