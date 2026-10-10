//! Cash count entity and kind.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountKind {
    Open,
    HandoverOut,
    Close,
    SafeOpen,
    Safe,
}

impl CountKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CountKind::Open => "OPEN",
            CountKind::HandoverOut => "HANDOVER_OUT",
            CountKind::Close => "CLOSE",
            CountKind::SafeOpen => "SAFE_OPEN",
            CountKind::Safe => "SAFE",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "OPEN" => Some(CountKind::Open),
            "HANDOVER_OUT" => Some(CountKind::HandoverOut),
            "CLOSE" => Some(CountKind::Close),
            "SAFE_OPEN" => Some(CountKind::SafeOpen),
            "SAFE" => Some(CountKind::Safe),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashCount {
    pub id: String,
    pub organization_id: String,
    pub branch_id: BranchId,
    pub shift_id: Option<String>,
    pub kind: CountKind,
    pub counted_by: String,
    pub counted_at: DateTime<Utc>,
    pub verified_by: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
