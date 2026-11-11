//! `PgAppSettingsRepository`: `customer_app_settings` (migration `20261106100000_customer_app.sql`).

use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;
use crate::domain::settings::{AppSettings, AppSettingsRepository, OtpChannel};

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
            let row: Option<(Option<i32>, Option<Vec<String>>)> = sqlx::query_as(
                "SELECT planner_draft_ttl_seconds, otp_channels FROM customer_app_settings \
                 WHERE organization_id = $1",
            )
            .bind(organization_id)
            .fetch_optional(&mut **tx)
            .await?;
            row.map(|(ttl, otp)| {
                let otp_channels = otp
                    .map(|list| {
                        // The column's CHECK admits only `email`/`sms`; anything else is a corrupt row.
                        list.iter()
                            .map(|s| OtpChannel::parse(s))
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(|| {
                                CustomerAppError::Server(format!("otp_channels holds {list:?}"))
                            })
                    })
                    .transpose()?;
                Ok(AppSettings {
                    organization_id,
                    planner_draft_ttl_seconds: ttl
                        .and_then(|s| u32::try_from(s).ok())
                        .and_then(NonZeroU32::new),
                    otp_channels,
                })
            })
            .transpose()
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
            let otp_channels: Option<Vec<&str>> = settings
                .otp_channels
                .as_ref()
                .map(|list| list.iter().map(|c| c.as_str()).collect());
            sqlx::query(
                "INSERT INTO customer_app_settings \
                     (organization_id, planner_draft_ttl_seconds, otp_channels, updated_by, updated_at) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (organization_id) DO UPDATE SET \
                     planner_draft_ttl_seconds = EXCLUDED.planner_draft_ttl_seconds, \
                     otp_channels = EXCLUDED.otp_channels, \
                     updated_by = EXCLUDED.updated_by, \
                     updated_at = EXCLUDED.updated_at",
            )
            .bind(settings.organization_id)
            .bind(ttl)
            .bind(otp_channels)
            .bind(updated_by)
            .bind(now)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    }
}
