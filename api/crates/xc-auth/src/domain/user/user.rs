//! User entity representing a Blueprint user.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::user_status::UserStatus;
use crate::domain::access::ScopedRole;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub organization_id: String,
    pub auth_subject: Option<String>,
    pub name: String,
    pub status: UserStatus,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub roles: Vec<ScopedRole>,
}
