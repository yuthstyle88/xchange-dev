//! Phone verification domain entity and status (CU-P1, design §6.3).

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PhoneVerificationStatus {
    Pending,
    Verified,
    Locked,
    Superseded,
}

#[derive(Debug, Clone)]
pub struct PhoneVerification {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub customer_id: String,
    pub branch_id: String,
    pub phone_encrypted: Vec<u8>,
    pub key_id: String,
    pub phone_lookup_hash: Vec<u8>,
    pub code_hash: Vec<u8>,
    pub expires_at: DateTime<Utc>,
    pub attempts: i32,
    pub status: PhoneVerificationStatus,
    pub requested_by: Uuid,
    pub identity_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

impl PhoneVerification {
    /// Computes code_hash = LookupHasher::hash(IdentityType::Phone, format!("xc:phone-otp:v1\u{1f}{challenge_id}\u{1f}{code}"))
    /// per lane controller ruling (a).
    pub fn try_compute_code_hash(challenge_id: Uuid, code: &str) -> AppResult<[u8; 32]> {
        let keys = crate::pii_keys::get_pii_keys()?;
        Ok(xc_kernel::pii::LookupHasher::new(&keys).hash(
            xc_kernel::pii::IdentityType::Phone,
            &format!("xc:phone-otp:v1\u{1f}{challenge_id}\u{1f}{code}"),
        ))
    }

    pub fn compute_code_hash(challenge_id: Uuid, code: &str) -> [u8; 32] {
        Self::try_compute_code_hash(challenge_id, code).expect("PII keys must be installed")
    }

    pub fn code_hash(challenge_id: Uuid, code: &str) -> [u8; 32] {
        Self::compute_code_hash(challenge_id, code)
    }
}
