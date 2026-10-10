//! R14: the limits are data — one `rate_alert_settings` row per organisation, saved by EXECUTIVE.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::source_policy::ObservePolicy;

/// One row of `rate_alert_settings` (design §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertSettings {
    pub organization_id: Uuid,
    pub default_cooldown_seconds: i32,
    /// Active alerts per customer, paused included, expired and deleted excluded (§4.1).
    pub max_active_per_customer: i32,
    /// `None` = no expiry by default.
    pub default_lifetime_days: Option<i32>,
    pub observe_policy: ObservePolicy,
    pub updated_by: Uuid,
    pub updated_at: DateTime<Utc>,
}

/// What `rateAlertSettingsSave` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAlertSettings {
    pub default_cooldown_seconds: i32,
    pub max_active_per_customer: i32,
    pub default_lifetime_days: Option<i32>,
    pub observe_policy: ObservePolicy,
}
