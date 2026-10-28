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
    /// Staff-corrections §2 SC-D7: set once a verifier rejects the count (`countReject`); `None` otherwise.
    pub rejection: Option<CountRejection>,
}

/// Who rejected a waiting count, when, and why (`cash_counts.rejected_by`, `rejected_at`, `reject_reason`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountRejection {
    pub by: String,
    pub at: DateTime<Utc>,
    pub reason: String,
}

/// A count's status, derived from the row (SC-D7): `VERIFIED` if verified, `REJECTED` if rejected, else `WAITING`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountStatus {
    Waiting,
    Verified,
    Rejected,
}

impl CountStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CountStatus::Waiting => "WAITING",
            CountStatus::Verified => "VERIFIED",
            CountStatus::Rejected => "REJECTED",
        }
    }
}

impl CashCount {
    pub fn status(&self) -> CountStatus {
        if self.verified_by.is_some() {
            CountStatus::Verified
        } else if self.rejection.is_some() {
            CountStatus::Rejected
        } else {
            CountStatus::Waiting
        }
    }
}
