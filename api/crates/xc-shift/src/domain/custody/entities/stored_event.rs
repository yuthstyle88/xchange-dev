//! Stored event entity for custody audit log.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::domain::custody::value_objects::custody_event::EventKind;
use crate::domain::custody::value_objects::custody_role::CustodyRole;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub id: String,
    pub kind: EventKind,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub role: CustodyRole,
    pub details: Value,
}
