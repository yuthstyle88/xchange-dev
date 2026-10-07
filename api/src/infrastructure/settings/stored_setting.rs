//! Stored setting, history entry, and version conflict types.

use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSetting {
    pub version: i64,
    pub body: Value,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingChange {
    pub version: i64,
    pub actor: String,
    pub at: DateTime<Utc>,
    pub before: Value,
    pub after: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionConflict {
    pub stored_version: i64,
}

impl VersionConflict {
    pub fn new(stored_version: i64) -> Self {
        Self { stored_version }
    }
}

impl fmt::Display for VersionConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "version conflict: stored version is {}",
            self.stored_version
        )
    }
}

impl std::error::Error for VersionConflict {}
