//! RA-2b (#1429): DTOs for rate alerts commands and views.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_rate_alerts.ts")]
pub struct CreateRateAlertInput {
    pub branch: String,
    pub currency: String,
    pub customer_side: String,
    pub condition: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_rate_alerts.ts")]
pub struct RateAlertLifecycleInput {
    #[serde(default)]
    pub id: Option<String>,
    #[ts(type = "number")]
    pub expected_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_rate_alerts.ts")]
pub struct RateAlertOutput {
    pub id: String,
    #[ts(type = "number")]
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_rate_alerts.ts")]
pub struct MyRateAlertItem {
    pub id: String,
    #[ts(type = "number")]
    pub version: i64,
    pub currency: String,
    pub customer_side: String,
    pub condition: String,
    pub target: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_rate_alerts.ts")]
pub struct MyRateAlertsData {
    pub alerts: Vec<MyRateAlertItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_alert_settings.ts")]
pub struct SaveRateAlertSettingsInput {
    pub default_cooldown_seconds: i32,
    pub max_active_per_customer: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_lifetime_days: Option<i32>,
    pub observe_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_alert_settings.ts")]
pub struct RateAlertSettingsData {
    pub default_cooldown_seconds: i32,
    pub max_active_per_customer: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_lifetime_days: Option<i32>,
    pub observe_policy: String,
}
