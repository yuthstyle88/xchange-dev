//! `PgKioskSettingsRepository` (design `kiosk-shared-phone.md` §4.2 D14, §4.6): `kiosk_settings`.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kiosk::domain::kiosk::entities::kiosk_settings::KioskSettings;
use xc_kiosk::domain::kiosk::repositories::kiosk_settings_repository::KioskSettingsRepository;

#[derive(Clone)]
pub struct PgKioskSettingsRepository {
    pool: PgPool,
}

impl PgKioskSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct KioskSettingsRow {
    branch_id: String,
    result_seconds: i32,
    updated_by: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    version: i64,
}

impl From<KioskSettingsRow> for KioskSettings {
    fn from(row: KioskSettingsRow) -> Self {
        Self {
            branch_id: BranchId::new(row.branch_id),
            result_seconds: row.result_seconds,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
            version: row.version,
        }
    }
}

impl KioskSettingsRepository for PgKioskSettingsRepository {
    async fn get(&self, branch: &BranchId) -> AppResult<Option<KioskSettings>> {
        let row = sqlx::query_as::<_, KioskSettingsRow>(
            "SELECT branch_id, result_seconds, updated_by, updated_at, version \
             FROM kiosk_settings WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Into::into))
    }

    async fn upsert_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        result_seconds: i32,
        by: &str,
        now: DateTime<Utc>,
    ) -> AppResult<KioskSettings> {
        let row = sqlx::query_as::<_, KioskSettingsRow>(
            "INSERT INTO kiosk_settings (branch_id, result_seconds, updated_by, updated_at, version) \
             VALUES ($1, $2, $3, $4, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET \
             result_seconds = EXCLUDED.result_seconds, \
             updated_by = EXCLUDED.updated_by, \
             updated_at = EXCLUDED.updated_at, \
             version = kiosk_settings.version + 1 \
             RETURNING branch_id, result_seconds, updated_by, updated_at, version",
        )
        .bind(branch.as_str())
        .bind(result_seconds)
        .bind(by)
        .bind(now)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.into())
    }
}
