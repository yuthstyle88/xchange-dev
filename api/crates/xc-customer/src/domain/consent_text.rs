//! Consent text domain entity (design `customer-identity-consent.md` §4.1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::consent::Purpose;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentText {
    pub purpose: Purpose,
    pub lang: String,
    pub version: i32,
    pub body: String,
    pub published_by: Option<String>,
    pub published_at: DateTime<Utc>,
}
