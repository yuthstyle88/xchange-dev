//! Account repository postgres implementation.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};

use crate::domain::account::entities::employee::Employee;
use crate::domain::account::entities::owner::Owner;
use crate::domain::account::repositories::account_repository::AccountRepository;

const EMPLOYEE_DATA: &str = "ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง";

#[derive(Debug, Clone)]
pub struct PgAccountRepository {
    pool: PgPool,
}

impl PgAccountRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AccountRepository for PgAccountRepository {
    async fn find_owner_by_username(&self, username: &str) -> AppResult<Option<(Owner, String)>> {
        let row_opt = sqlx::query(
            "SELECT id, username, name, password_hash, active, version, created_at FROM owners WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row_opt.map(|r| {
            let owner = Owner {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                active: r.get("active"),
                version: r.get("version"),
                created_at: r.get("created_at"),
            };
            let hash: String = r.get("password_hash");
            (owner, hash)
        }))
    }

    async fn find_owner_session(
        &self,
        token_hash: &str,
    ) -> AppResult<Option<(Owner, i32, DateTime<Utc>)>> {
        let row_opt = sqlx::query(
            r#"
            SELECT o.id, o.username, o.name, o.active, o.version, o.created_at,
                   s.version AS session_version, s.expires
            FROM owner_sessions s
            JOIN owners o ON s.owner_id = o.id
            WHERE s.hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row_opt.map(|r| {
            let owner = Owner {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                active: r.get("active"),
                version: r.get("version"),
                created_at: r.get("created_at"),
            };
            let session_version: i32 = r.get("session_version");
            let expires: DateTime<Utc> = r.get("expires");
            (owner, session_version, expires)
        }))
    }

    async fn create_owner_session(
        &self,
        token_hash: &str,
        owner_id: &str,
        version: i32,
        expires: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO owner_sessions (hash, owner_id, version, expires) VALUES ($1, $2, $3, $4)",
        )
        .bind(token_hash)
        .bind(owner_id)
        .bind(version)
        .bind(expires)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn delete_owner_session(&self, token_hash: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM owner_sessions WHERE hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    async fn find_employee_by_username(
        &self,
        username: &str,
    ) -> AppResult<Option<(Employee, String)>> {
        let row_opt = sqlx::query(
            "SELECT id, username, name, role, branch_id, password_hash, active, must_change, version, created_at FROM employees WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row_opt.map(|r| {
            let employee = Employee {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                role: r.get("role"),
                branch_id: r.get("branch_id"),
                active: r.get("active"),
                must_change: r.get("must_change"),
                version: r.get("version"),
                created_at: r.get("created_at"),
            };
            let hash: String = r.get("password_hash");
            (employee, hash)
        }))
    }

    async fn find_employee_by_id(&self, id: &str) -> AppResult<Option<Employee>> {
        let row_opt = sqlx::query(
            "SELECT id, username, name, role, branch_id, active, must_change, version, created_at FROM employees WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row_opt.map(|r| Employee {
            id: r.get("id"),
            username: r.get("username"),
            name: r.get("name"),
            role: r.get("role"),
            branch_id: r.get("branch_id"),
            active: r.get("active"),
            must_change: r.get("must_change"),
            version: r.get("version"),
            created_at: r.get("created_at"),
        }))
    }

    async fn find_employee_session(
        &self,
        token_hash: &str,
    ) -> AppResult<Option<(Employee, i32, DateTime<Utc>)>> {
        let row_opt = sqlx::query(
            r#"
            SELECT e.id, e.username, e.name, e.role, e.branch_id, e.active, e.must_change, e.version, e.created_at,
                   s.version AS session_version, s.expires
            FROM employee_sessions s
            JOIN employees e ON s.employee_id = e.id
            WHERE s.hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row_opt.map(|r| {
            let employee = Employee {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                role: r.get("role"),
                branch_id: r.get("branch_id"),
                active: r.get("active"),
                must_change: r.get("must_change"),
                version: r.get("version"),
                created_at: r.get("created_at"),
            };
            let session_version: i32 = r.get("session_version");
            let expires: DateTime<Utc> = r.get("expires");
            (employee, session_version, expires)
        }))
    }

    async fn create_employee_session(
        &self,
        token_hash: &str,
        employee_id: &str,
        version: i32,
        expires: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO employee_sessions (hash, employee_id, version, expires) VALUES ($1, $2, $3, $4)",
        )
        .bind(token_hash)
        .bind(employee_id)
        .bind(version)
        .bind(expires)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn delete_employee_session(&self, token_hash: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM employee_sessions WHERE hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    async fn list_employees(&self) -> AppResult<Vec<Employee>> {
        let rows = sqlx::query(
            "SELECT id, username, name, role, branch_id, active, must_change, version, created_at FROM employees ORDER BY username ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| Employee {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                role: r.get("role"),
                branch_id: r.get("branch_id"),
                active: r.get("active"),
                must_change: r.get("must_change"),
                version: r.get("version"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn create_employee(
        &self,
        employee: &Employee,
        password_hash: &str,
        actor: &str,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        let res = sqlx::query(
            r#"
            INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, must_change, version, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(&employee.id)
        .bind(&employee.username)
        .bind(&employee.name)
        .bind(&employee.role)
        .bind(&employee.branch_id)
        .bind(password_hash)
        .bind(employee.active)
        .bind(employee.must_change)
        .bind(employee.version)
        .bind(employee.created_at)
        .execute(&mut *tx)
        .await;

        if let Err(e) = res {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.code().as_deref() == Some("23505") {
                    return Err(AppError::Domain(EMPLOYEE_DATA.to_string()));
                }
            }
            return Err(AppError::Database(e));
        }

        sqlx::query(
            "INSERT INTO employee_audit (actor, subject, action, at) VALUES ($1, $2, 'CREATE', $3)",
        )
        .bind(actor)
        .bind(&employee.id)
        .bind(employee.created_at)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        tx.commit().await.map_err(AppError::Database)?;

        Ok(())
    }

    async fn reset_employee_password(
        &self,
        id: &str,
        password_hash: &str,
        now: DateTime<Utc>,
        actor: &str,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        sqlx::query(
            "UPDATE employees SET password_hash = $1, must_change = true, version = version + 1 WHERE id = $2",
        )
        .bind(password_hash)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO employee_audit (actor, subject, action, at) VALUES ($1, $2, 'RESET_PASSWORD', $3)",
        )
        .bind(actor)
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        tx.commit().await.map_err(AppError::Database)?;

        Ok(())
    }

    async fn disable_employee(&self, id: &str, now: DateTime<Utc>, actor: &str) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        sqlx::query("UPDATE employees SET active = false, version = version + 1 WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO employee_audit (actor, subject, action, at) VALUES ($1, $2, 'DISABLE', $3)",
        )
        .bind(actor)
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        tx.commit().await.map_err(AppError::Database)?;

        Ok(())
    }

    async fn change_employee_password(
        &self,
        id: &str,
        expected_version: i32,
        password_hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        let res = sqlx::query(
            "UPDATE employees SET password_hash = $1, must_change = false, version = version + 1 WHERE id = $2 AND version = $3",
        )
        .bind(password_hash)
        .bind(id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        if res.rows_affected() == 0 {
            return Ok(false);
        }

        sqlx::query(
            "INSERT INTO employee_audit (actor, subject, action, at) VALUES ($1, $2, 'CHANGE_PASSWORD', $3)",
        )
        .bind(id)
        .bind(id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        tx.commit().await.map_err(AppError::Database)?;

        Ok(true)
    }

    async fn active_employees(&self, branch_id: &str) -> AppResult<Vec<Employee>> {
        let rows = sqlx::query(
            "SELECT id, username, name, role, branch_id, active, must_change, version, created_at FROM employees WHERE branch_id = $1 AND active = true ORDER BY username ASC",
        )
        .bind(branch_id)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| Employee {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                role: r.get("role"),
                branch_id: r.get("branch_id"),
                active: r.get("active"),
                must_change: r.get("must_change"),
                version: r.get("version"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn create_owner(
        &self,
        id: &str,
        username: &str,
        name: &str,
        password_hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Owner> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        // Lock owners table to ensure single-owner atomicity under concurrency
        sqlx::query("LOCK TABLE owners IN EXCLUSIVE MODE")
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;

        let owner_count: i64 = sqlx::query_scalar("SELECT count(*) FROM owners")
            .fetch_one(&mut *tx)
            .await
            .map_err(AppError::Database)?;

        if owner_count > 0 {
            return Err(AppError::Domain("เจ้าของระบบมีอยู่แล้ว".to_string()));
        }

        let emp_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM employees WHERE username = $1")
                .bind(username)
                .fetch_one(&mut *tx)
                .await
                .map_err(AppError::Database)?;

        if emp_count > 0 {
            return Err(AppError::Domain("ชื่อบัญชีนี้ถูกใช้โดยพนักงานแล้ว".to_string()));
        }

        let row = sqlx::query(
            r#"
            INSERT INTO owners (id, username, name, password_hash, active, version, created_at)
            VALUES ($1, $2, $3, $4, true, 1, $5)
            RETURNING id, username, name, active, version, created_at
            "#,
        )
        .bind(id)
        .bind(username)
        .bind(name)
        .bind(password_hash)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.code().as_deref() == Some("23505") {
                    return AppError::Domain("เจ้าของระบบมีอยู่แล้ว".to_string());
                }
            }
            AppError::Database(e)
        })?;

        tx.commit().await.map_err(AppError::Database)?;

        Ok(Owner {
            id: row.get("id"),
            username: row.get("username"),
            name: row.get("name"),
            active: row.get("active"),
            version: row.get("version"),
            created_at: row.get("created_at"),
        })
    }

    async fn record_employee_login_audit(
        &self,
        employee_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO employee_audit (actor, subject, action, at) VALUES ($1, $1, 'LOGIN', $2)",
        )
        .bind(employee_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn increment_attempt(&self, key: &str, window_start: DateTime<Utc>) -> AppResult<i32> {
        let count: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO auth_attempts (key, window_start, count)
            VALUES ($1, $2, 1)
            ON CONFLICT (key, window_start)
            DO UPDATE SET count = auth_attempts.count + 1
            RETURNING count
            "#,
        )
        .bind(key)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(count)
    }
}
