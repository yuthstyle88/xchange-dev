//! RBAC §11.4: `RoleGrantStore` over `roles`, `role_permission_grants` and `role_grant_version` (migration M2).
//!
//! A permission is stored by its serde name (`"ExchangeTransact"`), the name `Me.permissions` already shows.
use std::collections::BTreeMap;

use sqlx::{PgConnection, Postgres, Transaction};

use crate::domain::access::{
    Group, Permission, Role, RoleGrantStore, RoleGrantStoreError, RoleGrantsSnapshot,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct PgRoleGrantStore;

impl PgRoleGrantStore {
    pub fn new() -> Self {
        Self
    }
}

fn db_err(e: impl std::fmt::Display) -> RoleGrantStoreError {
    RoleGrantStoreError::Database(e.to_string())
}

fn parse_perm(s: &str) -> Result<Permission, RoleGrantStoreError> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| RoleGrantStoreError::Database(format!("invalid stored permission {s:?}: {e}")))
}

impl RoleGrantStore for PgRoleGrantStore {
    async fn load(
        &self,
        conn: &mut PgConnection,
    ) -> Result<RoleGrantsSnapshot, RoleGrantStoreError> {
        let version = self.version(conn).await?;
        let role_rows: Vec<(String, String, String, bool)> =
            sqlx::query_as("SELECT code, name, base_role, built_in FROM roles ORDER BY code")
                .fetch_all(&mut *conn)
                .await
                .map_err(db_err)?;
        let grant_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT role_code, permission FROM role_permission_grants ORDER BY role_code, permission",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(db_err)?;

        let mut grants_by_role: BTreeMap<String, Vec<Permission>> = BTreeMap::new();
        for (role_code, perm_str) in grant_rows {
            let perm = parse_perm(&perm_str)?;
            grants_by_role.entry(role_code).or_default().push(perm);
        }

        let mut groups = Vec::with_capacity(role_rows.len());
        for (code, name, base_role, built_in) in role_rows {
            let base = Role::parse(&base_role).ok_or_else(|| {
                RoleGrantStoreError::Database(format!(
                    "invalid base_role {base_role:?} for group {code:?}"
                ))
            })?;
            let set = grants_by_role.remove(&code).unwrap_or_default();
            groups.push((
                Group {
                    code,
                    name,
                    base,
                    built_in,
                },
                set,
            ));
        }

        Ok(RoleGrantsSnapshot::new(version, groups))
    }

    async fn version(&self, conn: &mut PgConnection) -> Result<i64, RoleGrantStoreError> {
        sqlx::query_scalar("SELECT version FROM role_grant_version")
            .fetch_one(conn)
            .await
            .map_err(db_err)
    }

    async fn seed_missing(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<u64, RoleGrantStoreError> {
        let _ = tx;
        todo!("C6c")
    }

    async fn save_set(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code: &str,
        set: &[Permission],
    ) -> Result<(), RoleGrantStoreError> {
        let _ = (tx, code, set);
        todo!("C6c")
    }

    async fn add_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group: &Group,
        set: &[Permission],
    ) -> Result<(), RoleGrantStoreError> {
        let _ = (tx, group, set);
        todo!("C6c")
    }

    async fn delete_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code: &str,
    ) -> Result<(), RoleGrantStoreError> {
        let _ = (tx, code);
        todo!("C6c")
    }
}
