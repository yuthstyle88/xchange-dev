//! RBAC §11.4: `RoleGrantStore` over `roles`, `role_permission_grants` and `role_grant_version` (migration M2).
//!
//! A permission is stored by its serde name (`"ExchangeTransact"`), the name `Me.permissions` already shows.
//!
//! Skeleton (claude/65357afb, `feat/rbac-groups`): the bodies are C6c's.
use sqlx::{PgConnection, Postgres, Transaction};

use crate::domain::access::{
    Group, Permission, RoleGrantStore, RoleGrantStoreError, RoleGrantsSnapshot,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct PgRoleGrantStore;

impl PgRoleGrantStore {
    pub fn new() -> Self {
        Self
    }
}

impl RoleGrantStore for PgRoleGrantStore {
    async fn load(
        &self,
        conn: &mut PgConnection,
    ) -> Result<RoleGrantsSnapshot, RoleGrantStoreError> {
        let _ = conn;
        todo!("C6c")
    }

    async fn version(&self, conn: &mut PgConnection) -> Result<i64, RoleGrantStoreError> {
        let _ = conn;
        todo!("C6c")
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
