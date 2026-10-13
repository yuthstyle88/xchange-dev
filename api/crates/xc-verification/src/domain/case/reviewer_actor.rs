//! `ReviewerActor` (design V5, V17): a holder of `VerificationReview`, in the scope of their role row.

use uuid::Uuid;
use xc_kernel::ids::BranchId;

/// Where the reviewer's role row applies (`user_roles.branch_id`, V17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewerScope {
    /// `branch_id NULL`: the organisation's queue (app cases).
    Organization { organization_id: Uuid },
    /// A branch of the organisation (at-branch review, V17).
    Branch {
        organization_id: Uuid,
        branch_id: BranchId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewerActor {
    /// `users.id`.
    pub user_id: Uuid,
    pub scope: ReviewerScope,
}
