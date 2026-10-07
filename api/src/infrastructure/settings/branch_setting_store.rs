//! Per-branch versioned setting store implementation.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use serde_json::Value;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use super::stored_setting::{SettingChange, StoredSetting, VersionConflict};

#[derive(Clone)]
pub struct BranchSettingStore {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct SettingRow {
    version: i64,
    body: String,
    updated_at: DateTime<Utc>,
    updated_by: String,
}

impl SettingRow {
    fn try_into_stored(self) -> AppResult<StoredSetting> {
        let body: Value = serde_json::from_str(&self.body)
            .map_err(|e| pos_error::AppError::Internal(format!("corrupt setting body: {e}")))?;
        Ok(StoredSetting {
            version: self.version,
            body,
            updated_at: self.updated_at,
            updated_by: self.updated_by,
        })
    }
}

#[derive(sqlx::FromRow)]
struct HistoryRow {
    version: i64,
    actor: String,
    at: DateTime<Utc>,
    before: String,
    after: String,
}

impl HistoryRow {
    fn try_into_change(self) -> AppResult<SettingChange> {
        let before: Value = serde_json::from_str(&self.before)
            .map_err(|e| pos_error::AppError::Internal(format!("corrupt history before: {e}")))?;
        let after: Value = serde_json::from_str(&self.after)
            .map_err(|e| pos_error::AppError::Internal(format!("corrupt history after: {e}")))?;
        Ok(SettingChange {
            version: self.version,
            actor: self.actor,
            at: self.at,
            before,
            after,
        })
    }
}

impl BranchSettingStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn read(&self, branch: &BranchId, name: &str) -> AppResult<Option<StoredSetting>> {
        let row: Option<SettingRow> = sqlx::query_as(
            "SELECT version, body::text, updated_at, updated_by \
             FROM branch_settings \
             WHERE branch_id = $1 AND name = $2",
        )
        .bind(branch.as_str())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        row.map(|r| r.try_into_stored()).transpose()
    }

    pub async fn read_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        name: &str,
    ) -> AppResult<Option<StoredSetting>> {
        let row: Option<SettingRow> = sqlx::query_as(
            "SELECT version, body::text, updated_at, updated_by \
             FROM branch_settings \
             WHERE branch_id = $1 AND name = $2",
        )
        .bind(branch.as_str())
        .bind(name)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        row.map(|r| r.try_into_stored()).transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        name: &str,
        expected_version: i64,
        first_before: &serde_json::Value,
        body: &serde_json::Value,
        actor: &str,
        at: DateTime<Utc>,
    ) -> AppResult<Result<StoredSetting, VersionConflict>> {
        let current = self.read_in(tx, branch, name).await?;
        let current_version = current.as_ref().map_or(0, |s| s.version);

        if current_version != expected_version {
            return Ok(Err(VersionConflict::new(current_version)));
        }

        let new_version = expected_version + 1;
        let before_val = current.as_ref().map_or(first_before, |s| &s.body);
        let before_str = serde_json::to_string(before_val)
            .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
        let body_str = serde_json::to_string(body)
            .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;

        sqlx::query(
            "INSERT INTO branch_settings (branch_id, name, version, body, updated_at, updated_by) \
             VALUES ($1, $2, $3, $4::jsonb, $5, $6) \
             ON CONFLICT (branch_id, name) DO UPDATE SET \
                 version = EXCLUDED.version, \
                 body = EXCLUDED.body, \
                 updated_at = EXCLUDED.updated_at, \
                 updated_by = EXCLUDED.updated_by",
        )
        .bind(branch.as_str())
        .bind(name)
        .bind(new_version)
        .bind(&body_str)
        .bind(at)
        .bind(actor)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        sqlx::query(
            "INSERT INTO branch_setting_history (branch_id, name, version, actor, at, before, after) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb)",
        )
        .bind(branch.as_str())
        .bind(name)
        .bind(new_version)
        .bind(actor)
        .bind(at)
        .bind(&before_str)
        .bind(&body_str)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(Ok(StoredSetting {
            version: new_version,
            body: body.clone(),
            updated_at: at,
            updated_by: actor.to_string(),
        }))
    }

    pub async fn history(
        &self,
        branch: &BranchId,
        name: &str,
        limit: i64,
    ) -> AppResult<Vec<SettingChange>> {
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT version, actor, at, before::text, after::text \
             FROM branch_setting_history \
             WHERE branch_id = $1 AND name = $2 \
             ORDER BY version DESC LIMIT $3",
        )
        .bind(branch.as_str())
        .bind(name)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(pos_error::AppError::Database)?;

        rows.into_iter().map(|r| r.try_into_change()).collect()
    }
}
