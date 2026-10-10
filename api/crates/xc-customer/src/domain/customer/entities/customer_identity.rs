//! CustomerIdentity entity.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::pii::IdentityType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerIdentity {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub customer_id: String,
    pub r#type: IdentityType,
    pub encrypted_value: Vec<u8>,
    pub key_id: String,
    pub lookup_hash: Vec<u8>,
    pub nationality: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}
