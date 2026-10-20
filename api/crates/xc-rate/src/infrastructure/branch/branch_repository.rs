//! Branch repository postgres implementation.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::domain::branch::contact_channel::{BranchStatus, ContactChannel};
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
    serde_json::Value,
    String,
);

fn row_to_branch(row: BranchRow) -> Branch {
    let (
        id,
        name,
        request_id,
        code,
        address,
        timezone,
        opening_hours,
        booking_enabled,
        contact_channels,
        status,
    ) = row;
    let contact_channels: Vec<ContactChannel> =
        serde_json::from_value(contact_channels).unwrap_or_default();
    let status = BranchStatus::parse(&status).unwrap_or_default();
    Branch {
        id: BranchId::new(id),
        name,
        request_id,
        code,
        address,
        timezone,
        opening_hours,
        booking_enabled,
        contact_channels,
        status,
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
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled, contact_channels, status FROM branches ORDER BY (id = 'MAIN') DESC, created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(rows.into_iter().map(row_to_branch).collect())
    }

    async fn find_by_id(&self, id: &BranchId) -> AppResult<Option<Branch>> {
        let row = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled, contact_channels, status FROM branches WHERE id = $1",
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
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled, contact_channels, status FROM branches WHERE request_id = $1",
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

/// The profile writes of `branchSave` (OR-5a #945). Inherent, not on `BranchRepository`: they run
/// only inside a caller's transaction, which the trait's pool-based reads do not take.
impl PgBranchRepository {
    /// `id`'s branch read inside `tx`, locked `FOR UPDATE` until the caller commits.
    pub async fn find_for_update_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &BranchId,
    ) -> AppResult<Option<Branch>> {
        let row = sqlx::query_as::<_, BranchRow>(
            "SELECT id, name, request_id, code, address, timezone, opening_hours, booking_enabled, contact_channels, status FROM branches WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(row_to_branch))
    }

    /// Whether another branch of `id`'s organisation already holds `code`.
    pub async fn code_taken_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &BranchId,
        code: &str,
    ) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM branches o JOIN branches b ON b.id = $1 \
             WHERE o.organization_id IS NOT DISTINCT FROM b.organization_id AND o.code = $2 AND o.id <> b.id)",
        )
        .bind(id.as_str())
        .bind(code)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)
    }

    /// Writes `id`'s profile columns inside `tx`. A code another branch of the organisation took
    /// meanwhile (`branches_organization_id_code_key`) is `AppError::Conflict`.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_profile_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &BranchId,
        code: &str,
        address: Option<&str>,
        timezone: &str,
        opening_hours: Option<&serde_json::Value>,
        booking_enabled: bool,
        contact_channels: &[ContactChannel],
        status: BranchStatus,
    ) -> AppResult<()> {
        let channels_json =
            serde_json::to_value(contact_channels).unwrap_or_else(|_| serde_json::json!([]));
        sqlx::query(
            "UPDATE branches SET code = $2, address = $3, timezone = $4, opening_hours = $5, booking_enabled = $6, contact_channels = $7, status = $8 WHERE id = $1",
        )
        .bind(id.as_str())
        .bind(code)
        .bind(address)
        .bind(timezone)
        .bind(opening_hours)
        .bind(booking_enabled)
        .bind(channels_json)
        .bind(status.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.code().as_deref() == Some("23505") => {
                AppError::Conflict(format!("branch code {code:?} is taken in this organisation"))
            }
            _ => AppError::Database(e),
        })?;
        Ok(())
    }
}
