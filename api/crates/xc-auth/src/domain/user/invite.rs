//! Invite entity for user invites.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub user_id: String,
    pub code_hash: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

impl Invite {
    pub fn new(
        user_id: impl Into<String>,
        code_hash: impl Into<String>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            code_hash: code_hash.into(),
            expires_at,
            used_at: None,
        }
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }

    pub fn is_used(&self) -> bool {
        self.used_at.is_some()
    }

    pub fn can_claim(&self, now: DateTime<Utc>) -> bool {
        !self.is_used() && !self.is_expired(now)
    }
}
