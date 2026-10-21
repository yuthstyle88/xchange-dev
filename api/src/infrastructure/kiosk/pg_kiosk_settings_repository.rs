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

impl KioskSettingsRepository for PgKioskSettingsRepository {
    async fn get(&self, branch: &BranchId) -> AppResult<Option<KioskSettings>> {
        let _ = (&self.pool, branch);
        todo!("KWQ-E2: SELECT the branch's row")
    }

    async fn upsert_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        result_seconds: i32,
        by: &str,
        now: DateTime<Utc>,
    ) -> AppResult<KioskSettings> {
        let _ = (tx, branch, result_seconds, by, now);
        todo!("KWQ-E2: INSERT … ON CONFLICT (branch_id) DO UPDATE, version + 1")
    }
}
