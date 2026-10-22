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
                sqlx::query("UPDATE roles SET seed_version = $1 WHERE code = $2")
                    .bind(RoleGrants::SEED_VERSION)
                    .bind(code)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                seeded += 1;
            }
        }
        if seeded > 0 {
            bump_version(tx).await?;
        }
        Ok(seeded)
    }

    async fn seed_removals(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<u64, RoleGrantStoreError> {
        let mut deleted = 0u64;
        for role in RoleGrants::BUILT_IN {
            let code = role.as_str();
            let current_version: Option<i32> = sqlx::query_scalar(
                "SELECT seed_version FROM roles WHERE code = $1 AND built_in FOR UPDATE",
            )
            .bind(code)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?;
            let Some(current_version) = current_version else {
                continue;
            };
            if current_version >= RoleGrants::SEED_VERSION {
                continue;
            }
            let seed = RoleGrants::seed(role);
            for &(v, p) in RoleGrants::SEED_REMOVALS {
                if v > current_version && !seed.contains(&p) {
                    let name = perm_name(p)?;
                    let res = sqlx::query(
                        "DELETE FROM role_permission_grants WHERE role_code = $1 AND permission = $2",
                    )
                    .bind(code)
                    .bind(name)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                    deleted += res.rows_affected();
                }
            }
        }
        if deleted > 0 {
            bump_version(tx).await?;
        }
        Ok(deleted)
    }

    async fn seed_additions(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<u64, RoleGrantStoreError> {
        let mut inserted = 0u64;
        for role in RoleGrants::BUILT_IN {
            let code = role.as_str();
            let current_version: Option<i32> = sqlx::query_scalar(
                "SELECT seed_version FROM roles WHERE code = $1 AND built_in FOR UPDATE",
            )
            .bind(code)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?;
            let Some(current_version) = current_version else {
                continue;
            };
            if current_version >= RoleGrants::SEED_VERSION {
                continue;
            }
            let seed = RoleGrants::seed(role);
            for &(v, p) in RoleGrants::SEED_ADDITIONS {
                if v > current_version && seed.contains(&p) {
                    let name = perm_name(p)?;
                    let res = sqlx::query(
                        "INSERT INTO role_permission_grants (role_code, permission) \
                         VALUES ($1, $2) ON CONFLICT (role_code, permission) DO NOTHING",
                    )
                    .bind(code)
                    .bind(&name)
                    .execute(&mut **tx)
                    .await
                    .map_err(db_err)?;
                    inserted += res.rows_affected();
                    if v == 4 {
                        let res = sqlx::query(
                            "INSERT INTO role_permission_grants (role_code, permission) \
                             SELECT code, $2 FROM roles WHERE base_role = $1 AND NOT built_in \
                             ON CONFLICT (role_code, permission) DO NOTHING",
                        )
                        .bind(code)
                        .bind(&name)
                        .execute(&mut **tx)
                        .await
                        .map_err(db_err)?;
                        inserted += res.rows_affected();
                    }
                }
            }
            sqlx::query("UPDATE roles SET seed_version = $1 WHERE code = $2")
                .bind(RoleGrants::SEED_VERSION)
                .bind(code)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
        }
        if inserted > 0 {
            bump_version(tx).await?;
        }
        Ok(inserted)
    }

    async fn save_set(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code: &str,
        set: &[Permission],
    ) -> Result<(), RoleGrantStoreError> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roles WHERE code = $1)")
            .bind(code)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
        if !exists {
            return Err(RoleGrantStoreError::NotFound(code.to_owned()));
        }
        sqlx::query("DELETE FROM role_permission_grants WHERE role_code = $1")
            .bind(code)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        insert_grants(tx, code, set).await?;
        bump_version(tx).await
    }

    async fn add_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        group: &Group,
        set: &[Permission],
    ) -> Result<(), RoleGrantStoreError> {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roles WHERE code = $1)")
            .bind(&group.code)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
        if exists {
            return Err(RoleGrantStoreError::CodeTaken(group.code.clone()));
        }
        sqlx::query(
            "INSERT INTO roles (code, name, base_role, built_in) VALUES ($1, $2, $3, false)",
        )
        .bind(&group.code)
        .bind(&group.name)
        .bind(group.base.as_str())
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        insert_grants(tx, &group.code, set).await?;
        bump_version(tx).await
    }

    async fn delete_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        code: &str,
    ) -> Result<(), RoleGrantStoreError> {
        let built_in: Option<bool> =
            sqlx::query_scalar("SELECT built_in FROM roles WHERE code = $1 FOR UPDATE")
                .bind(code)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?;
        let Some(built_in) = built_in else {
            return Err(RoleGrantStoreError::NotFound(code.to_owned()));
        };
        if built_in {
            return Err(RoleGrantStoreError::BuiltIn(code.to_owned()));
        }
        let in_use: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_roles WHERE role_code = $1)")
                .bind(code)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)?;
        if in_use {
            return Err(RoleGrantStoreError::InUse(code.to_owned()));
        }
        sqlx::query("DELETE FROM roles WHERE code = $1")
            .bind(code)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        bump_version(tx).await
    }
}
