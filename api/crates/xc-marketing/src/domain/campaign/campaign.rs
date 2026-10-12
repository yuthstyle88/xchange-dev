//! `Campaign` (design §4.1 `campaigns`, §6).

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Campaign {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    /// Set when a version is APPROVED (D-3); `None` until then.
    pub current_version_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}
