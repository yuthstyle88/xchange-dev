//! RBAC §11.4: `RoleGrantStore` over `roles`, `role_permission_grants` and `role_grant_version` (migration M2).
//!
//! A permission is stored by its serde name (`"ExchangeTransact"`), the name `Me.permissions` already shows.
use std::collections::BTreeMap;

use sqlx::{PgConnection, Postgres, Transaction};

use crate::domain::access::{
    Group, Permission, Role, RoleGrantStore, RoleGrantStoreError, RoleGrants, RoleGrantsSnapshot,
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

fn perm_name(p: Permission) -> Result<String, RoleGrantStoreError> {
    match serde_json::to_value(p).map_err(db_err)? {
        serde_json::Value::String(s) => Ok(s),
        other => Err(RoleGrantStoreError::Database(format!(
            "permission {p:?} did not serialize to a string: {other}"
        ))),
    }
}

fn parse_perm(s: &str) -> Result<Permission, RoleGrantStoreError> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| RoleGrantStoreError::Database(format!("invalid stored permission {s:?}: {e}")))
}

async fn bump_version(tx: &mut Transaction<'_, Postgres>) -> Result<(), RoleGrantStoreError> {
    sqlx::query("UPDATE role_grant_version SET version = version + 1")
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn insert_grants(
    tx: &mut Transaction<'_, Postgres>,
    code: &str,
    set: &[Permission],
) -> Result<(), RoleGrantStoreError> {
    for &p in set {
        let name = perm_name(p)?;
        sqlx::query(
            "INSERT INTO role_permission_grants (role_code, permission) \
             VALUES ($1, $2) ON CONFLICT (role_code, permission) DO NOTHING",
        )
        .bind(code)
        .bind(name)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    }
    Ok(())
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
        let mut seeded = 0u64;
        for role in RoleGrants::BUILT_IN {
            let code = role.as_str();
            let has_rows: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM role_permission_grants WHERE role_code = $1)",
            )
            .bind(code)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
            if !has_rows {
                let seed = RoleGrants::seed(role);
                insert_grants(tx, code, &seed).await?;
                seeded += 1;
            }
        }
        if seeded > 0 {
            bump_version(tx).await?;
        }
        Ok(seeded)
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
