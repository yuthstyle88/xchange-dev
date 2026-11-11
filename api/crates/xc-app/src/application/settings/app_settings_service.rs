//! `AppSettingsService` (design §5): the only writer of `customer_app_settings` besides M1's seed.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use xc_auth::domain::access::actor::Actor;

use std::num::NonZeroU32;

use uuid::Uuid;
use xc_auth::domain::access::{Authorizer, Permission, Target};

use crate::domain::app_error::CustomerAppError;
use crate::domain::settings::{AppSettings, AppSettingsRepository, OtpChannel};
use crate::infrastructure::app_settings_repository::PgAppSettingsRepository;

pub struct AppSettingsService {
    pool: PgPool,
    settings: PgAppSettingsRepository,
}

impl AppSettingsService {
    /// Wires `PgAppSettingsRepository` itself.
    pub fn new(pool: PgPool) -> Self {
        Self {
            settings: PgAppSettingsRepository::new(pool.clone()),
            pool,
        }
    }

    /// `appSettingsSave` with only the draft lifetime: [`Self::save_patch_in`] keeping the OTP channels.
    pub async fn save_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        ttl_seconds: u32,
        now: DateTime<Utc>,
    ) -> Result<AppSettings, CustomerAppError> {
        self.save_patch_in(tx, actor, Some(ttl_seconds), None, now)
            .await
    }

    /// `appSettingsSave`: upserts `actor`'s organisation's row, `updated_by` = `actor.user_id`,
    /// `updated_at` = `now`. A `None` field keeps what is stored; `otp_channels: Some(None)` resets the
    /// channels to not set (#6286).
    ///
    /// - An actor without `Permission::SystemSettings` (ADMIN, EXECUTIVE) → `outOfScope`.
    /// - `ttl_seconds` = 0 → `validationFailed` on `plannerDraftTtlSeconds`.
    /// - A channel list that is empty, longer than 2, repeats or names an unknown channel →
    ///   `validationFailed` on `otpChannels`.
    pub async fn save_patch_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        ttl_seconds: Option<u32>,
        otp_channels: Option<Option<Vec<String>>>,
        now: DateTime<Utc>,
    ) -> Result<AppSettings, CustomerAppError> {
        // Any refusal of the rule is `outOfScope` here (ticket #994 §9), not `roleNotAllowed`.
        Authorizer::check(
            Some(actor),
            Permission::SystemSettings,
            &Target::organisation(&actor.organization_id),
            now,
        )
        .map_err(|_| CustomerAppError::OutOfScope)?;
        let ttl = ttl_seconds
            .map(|s| {
                NonZeroU32::new(s).ok_or(CustomerAppError::ValidationFailed {
                    field: "plannerDraftTtlSeconds",
                })
            })
            .transpose()?;
        let otp_channels = otp_channels
            .map(|list| list.map(|l| OtpChannel::parse_list(&l)).transpose())
            .transpose()?;
        let organization_id = Uuid::parse_str(&actor.organization_id).map_err(|_| {
            CustomerAppError::Server("the actor's organisation is not a uuid".into())
        })?;
        let stored = self.settings.get_in(tx, organization_id).await?;
        let settings = AppSettings {
            organization_id,
            planner_draft_ttl_seconds: ttl
                .or_else(|| stored.as_ref().and_then(|s| s.planner_draft_ttl_seconds)),
            otp_channels: otp_channels.unwrap_or_else(|| stored.and_then(|s| s.otp_channels)),
        };
        self.settings
            .upsert_in(tx, &settings, &actor.user_id, now)
            .await?;
        Ok(settings)
    }

    /// The OTP channels a customer of `organization_id` may choose, and `true` when the owner set them;
    /// the default when there is no row or the column is NULL (#6286).
    pub async fn otp_channels(
        &self,
        organization_id: Uuid,
    ) -> Result<(Vec<OtpChannel>, bool), CustomerAppError> {
        let mut tx = self.pool.begin().await?;
        let stored = self.settings.get_in(&mut tx, organization_id).await?;
        tx.commit().await?;
        Ok(OtpChannel::offered(stored.and_then(|s| s.otp_channels)))
    }
}
