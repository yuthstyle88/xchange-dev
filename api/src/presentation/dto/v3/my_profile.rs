//! CI-7 (#654): the `my_profile` view (design `customer-identity-consent.md` §4.4): a customer's own profile and
//! current consents. It holds no identity value, lookup hash or last 4 (K8).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::http::v3::view::ViewFilter;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct MyProfileData {
    pub customer_id: String,
    pub customer_no: String,
    pub display_name: String,
    /// `th`, `en` or `zh`.
    pub preferred_language: String,
    pub status: String,
    /// The languages among `th`, `en`, `zh`, in that order, in which the customer's organisation has a TERMS text (#4164).
    pub offered_languages: Vec<String>,
    /// The newest row per (purpose, channel), ordered by purpose, then channel.
    pub consents: Vec<MyConsent>,
    pub email_copy: EmailCopy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct EmailCopy {
    pub available: bool,
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct MyConsent {
    /// `TERMS` or `MARKETING`.
    pub purpose: String,
    /// `SMS`, `EMAIL` or `PUSH`; `null` for TERMS.
    pub channel: Option<String>,
    pub granted: bool,
    pub policy_version: i32,
    pub lang: String,
    /// RFC 3339.
    pub effective_at: String,
}

/// `?customer=`: the customer whose profile is asked for. Only the caller's own answers; any other is
/// `403 outOfScope` (the lane controller's ruling 7 of 2026-10-10 on #654). Absent means the caller's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "my_profile.ts")]
pub struct MyProfileFilter {
    pub customer: Option<String>,
}

impl ViewFilter for MyProfileFilter {}
