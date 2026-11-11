//! The organisation's app settings (owner C4: a planner draft lives 7 days, ADMIN can change it).
//! `customer_app_settings`, one row per organisation (design §4.1).

use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::app_error::CustomerAppError;

/// A channel a customer may receive a sign-in code on (#6286), stored as `email` / `sms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtpChannel {
    Email,
    Sms,
}

impl OtpChannel {
    /// The not-set fallback: email, then SMS, until the owner saves a list (#6286).
    pub const DEFAULT: [OtpChannel; 2] = [OtpChannel::Email, OtpChannel::Sms];

    pub fn as_str(self) -> &'static str {
        match self {
            OtpChannel::Email => "email",
            OtpChannel::Sms => "sms",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "email" => Some(OtpChannel::Email),
            "sms" => Some(OtpChannel::Sms),
            _ => None,
        }
    }

    /// An owner's list: 1–2 distinct known channels, order kept; else `validationFailed` on `otpChannels`.
    pub fn parse_list(list: &[String]) -> Result<Vec<Self>, CustomerAppError> {
        let invalid = CustomerAppError::ValidationFailed {
            field: "otpChannels",
        };
        let channels = list
            .iter()
            .map(|s| Self::parse(s))
            .collect::<Option<Vec<_>>>()
            .ok_or(invalid.clone())?;
        match channels.as_slice() {
            [_] => Ok(channels),
            [a, b] if a != b => Ok(channels),
            _ => Err(invalid),
        }
    }

    /// The stored list, or [`Self::DEFAULT`] when not set; `true` when the owner set it.
    pub fn offered(stored: Option<Vec<Self>>) -> (Vec<Self>, bool) {
        match stored {
            Some(set) => (set, true),
            None => (Self::DEFAULT.to_vec(), false),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub organization_id: Uuid,
    /// `planner_draft_ttl_seconds`; `None` = not set, no draft can be created.
    pub planner_draft_ttl_seconds: Option<NonZeroU32>,
    /// `otp_channels`, in the order the owner saved; `None` = not set, the default applies.
    pub otp_channels: Option<Vec<OtpChannel>>,
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
