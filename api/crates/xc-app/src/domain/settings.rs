//! The organisation's app settings (owner C4: a planner draft lives 7 days, ADMIN can change it).
//! `customer_app_settings`, one row per organisation (design §4.1).

use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub organization_id: Uuid,
    /// `planner_draft_ttl_seconds`; `None` = not set, no draft can be created.
    pub planner_draft_ttl_seconds: Option<NonZeroU32>,
}

impl AppSettings {
    /// `created_at` + the draft lifetime; `appSettingsMissing` when the lifetime is not set.
    pub fn draft_expires_at(
        &self,
        created_at: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, CustomerAppError> {
        let ttl = self
            .planner_draft_ttl_seconds
            .ok_or(CustomerAppError::AppSettingsMissing)?;
        Ok(created_at + chrono::Duration::seconds(i64::from(ttl.get())))
    }
}

/// The settings' storage port (design §5 `AppSettingsRepository`).
pub trait AppSettingsRepository: Send + Sync {
    /// The organisation's row; `None` when it has none.
    fn get_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, Result<Option<AppSettings>, CustomerAppError>>;

    /// `INSERT … ON CONFLICT (organization_id) DO UPDATE`.
    fn upsert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        settings: &'a AppSettings,
        updated_by: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>>;
}
