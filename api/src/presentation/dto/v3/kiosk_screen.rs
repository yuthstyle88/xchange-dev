//! #6273: DTOs of the kiosk screen values' view (`kiosk_screen`) and owner-only save (`kioskScreenSave`).
//!
//! The values themselves, their defaults and their bounds live in `application::kiosk_screen` only.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The server's not-set default values from `application::kiosk_screen`, so the admin screen can reset a field
/// without restating any default in TypeScript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_screen.ts")]
pub struct KioskScreenDefaults {
    pub quick_amounts: Vec<u32>,
    pub tiles: u32,
    pub idle_seconds: u32,
    pub review_seconds: u32,
    pub refresh_seconds: u32,
    pub popular_days: u32,
    pub default_payment_method: String,
}

/// A branch's kiosk screen values as the admin screen reads them: each value the kiosk is given, the fields
/// answering their default (`defaults`, so the screen can show "ค่าเริ่มต้น" beside them), the server's
/// `defaultValues` and enabled `paymentMethods`, and the row's `version` (`null` while the branch has no saved row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_screen.ts")]
pub struct KioskScreenData {
    pub branch: String,
    pub quick_amounts: Vec<u32>,
    pub tiles: u32,
    pub idle_seconds: u32,
    pub review_seconds: u32,
    pub refresh_seconds: u32,
    pub popular_days: u32,
    pub default_payment_method: String,
    pub defaults: Vec<String>,
    pub default_values: KioskScreenDefaults,
    pub payment_methods: Vec<String>,
    #[ts(type = "number | null")]
    pub version: Option<i64>,
}

/// The save: every field the owner sets. A field sent `null` (or left out) is not set, so it answers its
/// default ("กลับค่าเริ่มต้น"); the other fields keep what is sent. `expectedVersion` is the version read
/// (`null` while the branch has no row); any other answers `409 staleVersion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "kiosk_screen.ts")]
pub struct KioskScreenSaveInput {
    pub branch: String,
    #[serde(default)]
    #[ts(type = "Array<number> | null")]
    pub quick_amounts: Option<Vec<i64>>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub tiles: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub idle_seconds: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub review_seconds: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub refresh_seconds: Option<i64>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub popular_days: Option<i64>,
    #[serde(default)]
    pub default_payment_method: Option<String>,
    #[serde(default)]
    #[ts(type = "number | null")]
    pub expected_version: Option<i64>,
}
