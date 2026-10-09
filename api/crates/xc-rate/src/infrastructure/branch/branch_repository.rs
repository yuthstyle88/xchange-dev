//! Branch repository postgres implementation.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::domain::branch::entities::branch::Branch;
use crate::domain::branch::repositories::branch_repository::BranchRepository;

type BranchRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<serde_json::Value>,
    bool,
);

fn row_to_branch(row: BranchRow) -> Branch {
    let (id, name, request_id, code, address, timezone, opening_hours, booking_enabled) = row;
    Branch {
        id: BranchId::new(id),
        name,
        request_id,
        code,
        address,
        timezone,
        opening_hours,
        booking_enabled,
    }
}

#[derive(Clone)]
pub struct PgBranchRepository {
    pool: PgPool,
}

impl PgBranchRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchRepository for PgBranchRepository {
    async fn list(&self) -> AppResult<Vec<Branch>> {
        let rows = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled FROM branches ORDER BY (id = 'MAIN') DESC, created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(rows.into_iter().map(row_to_branch).collect())
    }

    async fn find_by_id(&self, id: &BranchId) -> AppResult<Option<Branch>> {
        let row = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled FROM branches WHERE id = $1",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(row_to_branch))
    }

    async fn branch_exists(&self, id: &BranchId) -> AppResult<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
            .bind(id.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)
    }

    async fn find_by_request(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        request_id: &str,
    ) -> AppResult<Option<Branch>> {
        let row = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled FROM branches WHERE request_id = $1",
        )
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(row_to_branch))
    }

    async fn is_abandoned(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        request_id: &str,
    ) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
        )
        .bind(branch_id)
        .bind(request_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)
    }

    async fn name_taken(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        name: &str,
    ) -> AppResult<bool> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE lower(name) = lower($1))")
            .bind(name)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)
    }

    async fn count(&self, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<i64> {
        sqlx::query_scalar("SELECT count(*) FROM branches")
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)
    }

    async fn insert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
        name: &str,
        created_at: DateTime<Utc>,
        created_by: &str,
        request_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO branches (id, name, created_at, created_by, request_id, code) VALUES ($1, $2, $3, $4, $5, $1)",
        )
        .bind(id)
        .bind(name)
        .bind(created_at)
        .bind(created_by)
        .bind(request_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }
}
