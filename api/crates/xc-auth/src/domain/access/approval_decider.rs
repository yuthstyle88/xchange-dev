//! RBAC §11.9 (design `docs/design/blueprint/user-management-rbac.md`): who decides each kind of approval.
//!
//! Every `ApprovalPolicy::may_decide` keeps every rule that is not about the role (proposer ≠ decider, the
//! organisation check, the transfer's limit split, `allows_self_decision`) and replaces its role test with one of
//! these two calls, passing the kind's permission (`permission_module::APPROVAL_KINDS`).
//!
//! - *Organisation* kinds (`ApproveCashTransferAboveLimit`, `ApproveCampaignVersion`, `ApproveRatePublication`,
//!   `ApproveLoyaltyRuleVersion`) call [`ApprovalDecider::holds_anywhere`]: any row qualifies, as today.
//! - *Branch* kinds (`ApproveCashTransferWithinLimit` on `from_branch`, `ApproveCountDifference`,
//!   `ApproveNegativeStock`, `ApproveExchangeTransaction`, `ApproveTransactionReversal`) call
//!   [`ApprovalDecider::holds_on`] with the request's branch: the row must also `covers` it.
//!
//! Neither reads a clock: the session holds only rows valid now (`session_service.rs`), the same assumption every
//! `may_decide` makes today. Neither reads the row's base role (`ScopedRole::role`): that is the point of §11.9.
//!
//! Skeleton (AP-0, BE lane controller): the bodies are AP-1's.
use super::actor::Actor;
use super::permission::Permission;

/// The shared core of the eight approval policies' role test (§11.9).
pub struct ApprovalDecider;

impl ApprovalDecider {
    /// True iff some row `r` of `who` has `RoleGrants::holds(r.group_code(), p)`, whatever its branch.
    /// The row's base role is not read.
    pub fn holds_anywhere(who: &Actor, p: Permission) -> bool {
        let _ = (who, p);
        todo!("AP-1")
    }

    /// True iff some row `r` of `who` has `RoleGrants::holds(r.group_code(), p)` and `r.covers(branch)`
    /// (`ScopedRole::covers`: an organisation-wide row covers every branch). The row's base role is not read.
    pub fn holds_on(who: &Actor, p: Permission, branch: Option<&str>) -> bool {
        let _ = (who, p, branch);
        todo!("AP-1")
    }
}
