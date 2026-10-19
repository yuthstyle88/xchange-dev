//! The rows the two audit views read (design §3.4, §4): `user_audit` (F4a `admin_user_audit`) and
//! `login_history` (F4b `admin_login_history`). `UserRepository::list_audit` / `list_logins` (C1c)
//! return them newest first.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One `user_audit` row, with the actor's and the subject's names joined from `users`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserAuditEntry {
    pub id: String,
    pub at: DateTime<Utc>,
    pub actor_id: String,
    pub actor_name: String,
    pub subject_id: String,
    pub subject_name: String,
    /// As written by `AccountAuditor::record`: `userGrant`, `userRevoke`, `userDisable`,
    /// `userSuspend`, `userResume`, `userEnable`, `userProfileSave`, and the create actions.
    pub action: String,
    /// NULL on rows written before migration M.
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub ip: Option<String>,
}

/// One `login_history` row. `user_id`/`user_name` are `None` when the identifier matched no user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginEntry {
    pub id: String,
    pub at: DateTime<Utc>,
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub ip: Option<String>,
    /// `OK`, `BAD_PASSWORD`, `UNKNOWN_USER`, `NOT_ACTIVE`, `TEMP_EXPIRED`, `RATE_LIMITED` or `UNAVAILABLE`.
    pub result: String,
}
