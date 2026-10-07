//! PostgreSQL implementation of the device repository.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use super::device_event::DeviceEvent;
use super::device_job::{DeviceJob, JobKind, JobStatus};
use super::device_repository::{AMOUNT_KEYS, DeviceRepository, NewEvent, NewJob, StoredJob};

#[derive(Clone)]
pub struct PgDeviceRepository {
    pool: PgPool,
}

impl PgDeviceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const JOB_COLUMNS: &str =
    "id, request_id, request_hash, branch_id, ref, kind, status, actor, created_at, updated_at";

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    request_id: String,
    request_hash: String,
    branch_id: String,
    #[sqlx(rename = "ref")]
    reference: String,
    kind: String,
    status: String,
    actor: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl JobRow {
    fn into_stored(self) -> AppResult<StoredJob> {
        let kind = JobKind::parse(&self.kind).ok_or_else(|| {
            AppError::Internal(format!("device job {}: kind {}", self.id, self.kind))
        })?;
        let status = JobStatus::parse(&self.status).ok_or_else(|| {
            AppError::Internal(format!("device job {}: status {}", self.id, self.status))
        })?;
        Ok(StoredJob {
            job: DeviceJob {
                id: self.id,
                request_id: self.request_id,
                branch_id: self.branch_id,
                reference: self.reference,
                kind,
                status,
                actor: self.actor,
                created_at: self.created_at,
                updated_at: self.updated_at,
            },
            request_hash: self.request_hash,
        })
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    seq: i64,
    branch_id: String,
    actor: String,
    kind: String,
    #[sqlx(rename = "ref")]
    reference: String,
    detail: String,
    at: DateTime<Utc>,
}

impl EventRow {
    fn into_event(self) -> AppResult<DeviceEvent> {
        Ok(DeviceEvent {
            seq: self.seq,
            branch_id: self.branch_id,
            actor: self.actor,
            kind: self.kind,
            reference: self.reference,
            detail: serde_json::from_str(&self.detail)?,
            at: self.at,
        })
    }
}

impl DeviceRepository for PgDeviceRepository {
    async fn list_jobs(&self, branch: &BranchId, limit: i64) -> AppResult<Vec<DeviceJob>> {
        let rows: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {JOB_COLUMNS} FROM device_jobs WHERE branch_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2"
        )))
        .bind(branch.as_str())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| row.into_stored().map(|stored| stored.job))
            .collect()
    }

    async fn list_events(&self, branch: &BranchId, limit: i64) -> AppResult<Vec<DeviceEvent>> {
        let rows: Vec<EventRow> = sqlx::query_as(
            "SELECT seq, branch_id, actor, kind, ref, \
             (detail || COALESCE((SELECT jsonb_object_agg(a.key, detail->>a.key) \
                                  FROM unnest($3::text[]) AS a(key) \
                                  WHERE jsonb_typeof(detail->a.key) = 'number'), '{}'::jsonb))::text AS detail, \
             at \
             FROM device_events WHERE branch_id = $1 ORDER BY seq DESC LIMIT $2",
        )
        .bind(branch.as_str())
        .bind(limit)
        .bind(AMOUNT_KEYS.as_slice())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(EventRow::into_event).collect()
    }

    async fn setting(&self, branch: &BranchId, key: &str) -> AppResult<Option<String>> {
        let value = sqlx::query_scalar(
            "SELECT value FROM device_settings WHERE branch_id = $1 AND key = $2",
        )
        .bind(branch.as_str())
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(value)
    }

    async fn setting_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        key: &str,
    ) -> AppResult<Option<String>> {
        let value = sqlx::query_scalar(
            "SELECT value FROM device_settings WHERE branch_id = $1 AND key = $2",
        )
        .bind(branch.as_str())
        .bind(key)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(value)
    }

    async fn put_setting(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        key: &str,
        value: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, $2, $3) \
             ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(branch.as_str())
        .bind(key)
        .bind(value)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn insert_job(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        job: &NewJob<'_>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "INSERT INTO device_jobs \
             (id, request_id, request_hash, branch_id, ref, kind, status, actor, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) \
             ON CONFLICT (branch_id, request_id) DO NOTHING",
        )
        .bind(job.id)
        .bind(job.request_id)
        .bind(job.request_hash)
        .bind(branch.as_str())
        .bind(job.reference)
        .bind(job.kind.as_str())
        .bind(JobStatus::Queued.as_str())
        .bind(job.actor)
        .bind(job.at)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    async fn find_job_by_request(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
    ) -> AppResult<Option<StoredJob>> {
        let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {JOB_COLUMNS} FROM device_jobs WHERE branch_id = $1 AND request_id = $2"
        )))
        .bind(branch.as_str())
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?;
        row.map(JobRow::into_stored).transpose()
    }

    async fn find_job(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        id: &str,
    ) -> AppResult<Option<DeviceJob>> {
        let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {JOB_COLUMNS} FROM device_jobs WHERE branch_id = $1 AND id = $2"
        )))
        .bind(branch.as_str())
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row
            .map(JobRow::into_stored)
            .transpose()?
            .map(|stored| stored.job))
    }

    async fn set_job_status(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        id: &str,
        status: JobStatus,
        at: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE device_jobs SET status = $1, updated_at = $2 WHERE branch_id = $3 AND id = $4",
        )
        .bind(status.as_str())
        .bind(at)
        .bind(branch.as_str())
        .bind(id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn insert_event(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        event: &NewEvent<'_>,
    ) -> AppResult<bool> {
        let done = sqlx::query(
            "INSERT INTO device_events (dedupe, branch_id, actor, kind, ref, detail, at) \
             VALUES ($1, $2, $3, $4, $5, $6::jsonb || $8::jsonb, $7) \
             ON CONFLICT (branch_id, dedupe) DO NOTHING",
        )
        .bind(event.dedupe)
        .bind(branch.as_str())
        .bind(event.actor)
        .bind(event.kind)
        .bind(event.reference)
        .bind(event.detail.to_string())
        .bind(event.at)
        .bind(amounts_json(event.amounts)?)
        .execute(&mut **tx)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}

/// `{"thb":12345669998.765433}` — the amounts as a JSON object written digit
/// by digit, for PostgreSQL to read as exact numbers. A decimal prints as
/// plain digits with an optional sign and point, which is a JSON number.
fn amounts_json(amounts: &[(&'static str, Decimal)]) -> AppResult<String> {
    let mut text = String::from("{");
    for (i, (key, amount)) in amounts.iter().enumerate() {
        if !AMOUNT_KEYS.contains(key) {
            return Err(AppError::Internal(format!(
                "device line: {key} is not an amount key"
            )));
        }
        if i > 0 {
            text.push(',');
        }
        text.push_str(&serde_json::to_string(key)?);
        text.push(':');
        text.push_str(&amount.normalize().to_string());
    }
    text.push('}');
    Ok(text)
}
