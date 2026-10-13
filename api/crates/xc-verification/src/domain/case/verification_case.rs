//! `VerificationCase` (design V1, §4.1 `verification_cases`).

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

use super::case_status::CaseStatus;
use super::decision_reason::DecisionReason;
use super::provider::Provider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationCase {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub customer_id: String,
    pub provider: Provider,
    pub status: CaseStatus,
    /// The policy version the case was opened under (V11).
    pub policy_version: i32,
    /// `NULL` = an app case (organisation queue); a branch for an at-branch case (V17).
    pub branch_id: Option<BranchId>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub decided_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub reviewed_by: Option<Uuid>,
    pub decision_reason: Option<DecisionReason>,
    /// `expectedVersion` (V5).
    pub version: i32,
}
