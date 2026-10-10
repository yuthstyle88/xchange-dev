//! PostgreSQL implementation of UserRepository.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Row, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::access::{Role, ScopedRole};
use crate::domain::user::user::User;
use crate::domain::user::user_repository::UserRepository;
use crate::domain::user::user_status::UserStatus;

#[derive(Debug, Clone, Copy, Default)]
pub struct PgUserRepository;

impl PgUserRepository {
    pub fn new() -> Self {
        Self
    }
}

impl UserRepository for PgUserRepository {
    async fn find_by_id(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
    ) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT id::text, organization_id::text, auth_subject, name, status, version, created_at, created_by, legacy_employee_id
            FROM users
            WHERE id = $1::uuid
            "#,
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let user_id: String = row.get("id");
        let roles = self.find_roles_by_user_id(tx, &user_id).await?;
        let status_str: String = row.get("status");
        let status = UserStatus::parse(&status_str).unwrap_or(UserStatus::Pending);

        Ok(Some(User {
            id: user_id,
            organization_id: row.get("organization_id"),
            auth_subject: row.get("auth_subject"),
            name: row.get("name"),
            status,
            version: row.get("version"),
            created_at: row.get("created_at"),
            created_by: row.get("created_by"),
            legacy_employee_id: row.get("legacy_employee_id"),
            roles,
        }))
    }

    async fn find_by_auth_subject(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        subject: &str,
    ) -> AppResult<Option<User>> {
        let row = sqlx::query(
            r#"
            SELECT id::text, organization_id::text, auth_subject, name, status, version, created_at, created_by, legacy_employee_id
            FROM users
            WHERE auth_subject = $1
            "#,
        )
        .bind(subject)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let user_id: String = row.get("id");
        let roles = self.find_roles_by_user_id(tx, &user_id).await?;
        let status_str: String = row.get("status");
        let status = UserStatus::parse(&status_str).unwrap_or(UserStatus::Pending);

        Ok(Some(User {
            id: user_id,
            organization_id: row.get("organization_id"),
            auth_subject: row.get("auth_subject"),
            name: row.get("name"),
            status,
            version: row.get("version"),
            created_at: row.get("created_at"),
            created_by: row.get("created_by"),
            legacy_employee_id: row.get("legacy_employee_id"),
            roles,
        }))
    }

    async fn create_user(&self, tx: &mut Transaction<'_, Postgres>, user: &User) -> AppResult<()> {
        sqlx::query(
            r#"
            INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by, legacy_employee_id)
            VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(&user.id)
        .bind(&user.organization_id)
        .bind(&user.auth_subject)
        .bind(&user.name)
        .bind(user.status.as_str())
        .bind(user.version)
        .bind(user.created_at)
        .bind(&user.created_by)
        .bind(&user.legacy_employee_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn set_status(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        status: UserStatus,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            UPDATE users
            SET status = $1, version = version + 1
            WHERE id = $2::uuid
            "#,
        )
        .bind(status.as_str())
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn append_audit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor_id: &str,
        subject_id: &str,
        action: &str,
        organization_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO user_audit (actor_id, subject_id, action, organization_id) \
             VALUES ($1, $2, $3, $4::uuid)",
        )
        .bind(actor_id)
        .bind(subject_id)
        .bind(action)
        .bind(organization_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_auth_subject(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        auth_subject: &str,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            UPDATE users
            SET auth_subject = $1
            WHERE id = $2::uuid
            "#,
        )
        .bind(auth_subject)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn bump_version(&self, tx: &mut Transaction<'_, Postgres>, id: &str) -> AppResult<i32> {
        let version: i32 = sqlx::query_scalar(
            r#"
            UPDATE users
            SET version = version + 1
            WHERE id = $1::uuid
            RETURNING version
            "#,
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(version)
    }

    async fn add_role(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        organization_id: &str,
        role: &ScopedRole,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            INSERT INTO user_roles (id, user_id, role_code, organization_id, branch_id, valid_from, valid_to)
            VALUES (gen_random_uuid(), $1::uuid, $2, $3::uuid, $4, $5, $6)
            "#,
        )
        .bind(user_id)
        .bind(role.role.as_str())
        .bind(organization_id)
        .bind(role.branch.as_ref().map(|b| b.as_str()))
        .bind(role.valid_from)
        .bind(role.valid_to)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn revoke_role(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        role: Role,
        branch: Option<&BranchId>,
        valid_to: DateTime<Utc>,
    ) -> AppResult<()> {
        let branch_str = branch.map(|b| b.as_str());
        sqlx::query(
            r#"
            UPDATE user_roles
            SET valid_to = $4
            WHERE user_id = $1::uuid
              AND role_code = $2
              AND ((branch_id IS NULL AND $3::text IS NULL) OR branch_id = $3::text)
              AND (valid_to IS NULL OR valid_to > $4)
            "#,
        )
        .bind(user_id)
        .bind(role.as_str())
        .bind(branch_str)
        .bind(valid_to)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn find_roles_by_user_id(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
    ) -> AppResult<Vec<ScopedRole>> {
        let rows = sqlx::query(
            r#"
            SELECT role_code, branch_id, valid_from, valid_to
            FROM user_roles
            WHERE user_id = $1::uuid
            ORDER BY valid_from ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let mut roles = Vec::with_capacity(rows.len());
        for row in rows {
            let role_code: String = row.get("role_code");
            let role = Role::parse(&role_code)
                .ok_or_else(|| AppError::Internal(format!("unknown role code {role_code}")))?;
            let branch_id: Option<String> = row.get("branch_id");
            let branch = branch_id.map(BranchId::new);
            let valid_from: DateTime<Utc> = row.get("valid_from");
            let valid_to: Option<DateTime<Utc>> = row.get("valid_to");
            roles.push(ScopedRole::new(role, branch, valid_from, valid_to));
        }

        Ok(roles)
    }
}
