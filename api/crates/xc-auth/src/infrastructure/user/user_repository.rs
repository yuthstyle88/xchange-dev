//! PostgreSQL implementation of UserRepository.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Row, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::access::{Role, ScopedRole};
use crate::domain::user::user::User;
use crate::domain::user::user_history::{LoginEntry, UserAuditEntry};
use crate::domain::user::user_profile::UserProfile;
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
        let status = UserStatus::parse(&status_str)
            .ok_or_else(|| AppError::Internal(format!("unknown user status {status_str}")))?;

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
        let status = UserStatus::parse(&status_str)
            .ok_or_else(|| AppError::Internal(format!("unknown user status {status_str}")))?;

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
        .bind(role.group_code())
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
        role_code: &str,
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
        .bind(role_code)
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
            SELECT ur.role_code, r.base_role, ur.branch_id, ur.valid_from, ur.valid_to
            FROM user_roles ur
            JOIN roles r ON r.code = ur.role_code
            WHERE ur.user_id = $1::uuid
            ORDER BY ur.valid_from ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let mut roles = Vec::with_capacity(rows.len());
        for row in rows {
            let role_code: String = row.get("role_code");
            let base_role: String = row.get("base_role");
            let role = Role::parse(&base_role)
                .ok_or_else(|| AppError::Internal(format!("unknown role code {base_role}")))?;
            let branch_id: Option<String> = row.get("branch_id");
            let branch = branch_id.map(BranchId::new);
            let valid_from: DateTime<Utc> = row.get("valid_from");
            let valid_to: Option<DateTime<Utc>> = row.get("valid_to");
            roles.push(ScopedRole::in_group(
                &role_code, role, branch, valid_from, valid_to,
            ));
        }

        Ok(roles)
    }

    async fn count_active_owners(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
    ) -> AppResult<usize> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT count(DISTINCT u.id)
            FROM users u
            JOIN user_roles r ON r.user_id = u.id
            LEFT JOIN organizations o ON o.id = u.organization_id
            WHERE (u.organization_id::text = $1 OR o.code = $1)
              AND u.status = 'ACTIVE'
              AND r.role_code IN ('EXECUTIVE', 'ADMIN')
              AND r.branch_id IS NULL
              AND r.valid_from <= now()
              AND (r.valid_to IS NULL OR r.valid_to > now())
            "#,
        )
        .bind(organization_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(count as usize)
    }

    async fn find_profile(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
    ) -> AppResult<Option<UserProfile>> {
        let row = sqlx::query(
            r#"
            SELECT employee_code, department, position, tell_code
            FROM users
            WHERE id = $1::uuid
            "#,
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(|r| UserProfile {
            employee_code: r.get("employee_code"),
            department: r.get("department"),
            position: r.get("position"),
            tell_code: r.get("tell_code"),
        }))
    }

    async fn set_profile(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
        profile: &UserProfile,
    ) -> AppResult<()> {
        fn norm(s: &Option<String>) -> Option<&str> {
            s.as_deref().map(str::trim).filter(|v| !v.is_empty())
        }
        let res = sqlx::query(
            r#"
            UPDATE users
            SET employee_code = $1,
                department = $2,
                position = $3,
                tell_code = $4
            WHERE id = $5::uuid
            "#,
        )
        .bind(norm(&profile.employee_code))
        .bind(norm(&profile.department))
        .bind(norm(&profile.position))
        .bind(norm(&profile.tell_code))
        .bind(id)
        .execute(&mut **tx)
        .await;

        if let Err(e) = res {
            if let sqlx::Error::Database(ref db_err) = e
                && db_err.code().as_deref() == Some("23505")
                && db_err.constraint() == Some("users_org_employee_code_key")
            {
                return Err(AppError::Conflict("employeeCodeTaken".into()));
            }
            return Err(AppError::Database(e));
        }

        Ok(())
    }

    async fn list_audit(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
        before: Option<&str>,
        limit: u16,
    ) -> AppResult<Vec<UserAuditEntry>> {
        if before.is_some_and(|b| !is_uuid(b)) {
            // A malformed cursor reads as "nothing older" (#3213); it never reaches the `::uuid` cast.
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            r#"
            SELECT a.id::text AS id,
                   a.at,
                   a.actor_id,
                   COALESCE(actor.name, a.actor_id) AS actor_name,
                   a.subject_id,
                   COALESCE(subj.name, a.subject_id) AS subject_name,
                   a.action,
                   a.before,
                   a.after,
                   host(a.ip) AS ip
            FROM user_audit a
            LEFT JOIN users actor ON actor.id::text = a.actor_id
            LEFT JOIN users subj ON subj.id::text = a.subject_id
            WHERE a.organization_id = $1::uuid
              AND ($2::uuid IS NULL OR (a.at, a.id) < (SELECT at, id FROM user_audit WHERE id = $2::uuid))
            ORDER BY a.at DESC, a.id DESC
            LIMIT $3
            "#,
        )
        .bind(organization_id)
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| UserAuditEntry {
                id: r.get("id"),
                at: r.get("at"),
                actor_id: r.get("actor_id"),
                actor_name: r.get("actor_name"),
                subject_id: r.get("subject_id"),
                subject_name: r.get("subject_name"),
                action: r.get("action"),
                before: r.get("before"),
                after: r.get("after"),
                ip: r.get("ip"),
            })
            .collect())
    }

    async fn list_logins(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
        before: Option<&str>,
        limit: u16,
    ) -> AppResult<Vec<LoginEntry>> {
        if before.is_some_and(|b| !is_uuid(b)) {
            // A malformed cursor reads as "nothing older" (#3213); it never reaches the `::uuid` cast.
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            r#"
            SELECT l.id::text AS id,
                   l.at,
                   l.user_id::text AS user_id,
                   u.name AS user_name,
                   host(l.ip) AS ip,
                   l.result
            FROM login_history l
            LEFT JOIN users u ON u.id = l.user_id
            WHERE l.organization_id = $1::uuid
              AND ($2::uuid IS NULL OR (l.at, l.id) < (SELECT at, id FROM login_history WHERE id = $2::uuid))
            ORDER BY l.at DESC, l.id DESC
            LIMIT $3
            "#,
        )
        .bind(organization_id)
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| LoginEntry {
                id: r.get("id"),
                at: r.get("at"),
                user_id: r.get("user_id"),
                user_name: r.get("user_name"),
                ip: r.get("ip"),
                result: r.get("result"),
            })
            .collect())
    }
}

/// `8-4-4-4-12` hex digits, the text form of a uuid: checked before a client-supplied cursor is cast (#3213).
fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}
