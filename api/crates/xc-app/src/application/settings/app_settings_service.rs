//! `AppSettingsService` (design §5): the only writer of `customer_app_settings` besides M1's seed.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use xc_auth::domain::access::actor::Actor;

use std::num::NonZeroU32;

use uuid::Uuid;
use xc_auth::domain::access::{Authorizer, Permission, Target};

use crate::domain::app_error::CustomerAppError;
use crate::domain::settings::{AppSettings, AppSettingsRepository};
use crate::infrastructure::app_settings_repository::PgAppSettingsRepository;

pub struct AppSettingsService {
    settings: PgAppSettingsRepository,
}

impl AppSettingsService {
    /// Wires `PgAppSettingsRepository` itself.
    pub fn new(pool: PgPool) -> Self {
        Self {
            settings: PgAppSettingsRepository::new(pool),
        }
    }

    /// `appSettingsSave`: upserts the draft lifetime of `actor`'s organisation, `updated_by` =
    /// `actor.user_id`, `updated_at` = `now`.
    ///
    /// - An actor without `Permission::SystemSettings` (ADMIN, EXECUTIVE) → `outOfScope`.
    /// - `ttl_seconds` = 0 → `validationFailed` on `plannerDraftTtlSeconds`.
    pub async fn save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        ttl_seconds: u32,
        now: DateTime<Utc>,
    ) -> Result<AppSettings, CustomerAppError> {
        // Any refusal of the rule is `outOfScope` here (ticket #994 §9), not `roleNotAllowed`.
        Authorizer::check(
            Some(actor),
            Permission::SystemSettings,
            &Target::new(&actor.organization_id, None, None),
            now,
        )
        .map_err(|_| CustomerAppError::OutOfScope)?;
        let ttl = NonZeroU32::new(ttl_seconds).ok_or(CustomerAppError::ValidationFailed {
            field: "plannerDraftTtlSeconds",
        })?;
        let organization_id = Uuid::parse_str(&actor.organization_id).map_err(|_| {
            CustomerAppError::Server("the actor's organisation is not a uuid".into())
        })?;
        let settings = AppSettings {
            organization_id,
            planner_draft_ttl_seconds: Some(ttl),
        };
        self.settings
            .upsert_in(tx, &settings, &actor.user_id, now)
            .await?;
        Ok(settings)
    }
}
