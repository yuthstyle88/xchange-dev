//! Receipt record entity.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub id: Uuid,
    pub number: String,
    pub revision: i32,
    pub issued_at: DateTime<Utc>,
    pub document_ref: String,
}
