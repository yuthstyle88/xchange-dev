//! `PgAppSettingsRepository`: `customer_app_settings` (migration `20261106100000_customer_app.sql`).

use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;
use crate::domain::settings::{AppSettings, AppSettingsRepository};

pub struct PgAppSettingsRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgAppSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AppSettingsRepository for PgAppSettingsRepository {
    fn get_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<AppSettings>, CustomerAppError>> {
        Box::pin(async move {
            let ttl: Option<Option<i32>> = sqlx::query_scalar(
                "SELECT planner_draft_ttl_seconds FROM customer_app_settings \
                 WHERE organization_id = $1",
            )
            .bind(organization_id)
            .fetch_optional(&mut **tx)
            .await?;
            Ok(ttl.map(|ttl| AppSettings {
                organization_id,
                planner_draft_ttl_seconds: ttl
                    .and_then(|s| u32::try_from(s).ok())
                    .and_then(NonZeroU32::new),
            }))
        })
    }

    fn upsert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        settings: &'a AppSettings,
        updated_by: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>> {
        Box::pin(async move {
            let ttl = settings
                .planner_draft_ttl_seconds
                .map(|s| i32::try_from(s.get()))
                .transpose()
                .map_err(|_| CustomerAppError::ValidationFailed {
                    field: "plannerDraftTtlSeconds",
                })?;
            sqlx::query(
                "INSERT INTO customer_app_settings \
                     (organization_id, planner_draft_ttl_seconds, updated_by, updated_at) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (organization_id) DO UPDATE SET \
                     planner_draft_ttl_seconds = EXCLUDED.planner_draft_ttl_seconds, \
                     updated_by = EXCLUDED.updated_by, \
                     updated_at = EXCLUDED.updated_at",
            )
            .bind(settings.organization_id)
            .bind(ttl)
            .bind(updated_by)
            .bind(now)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    }
}
