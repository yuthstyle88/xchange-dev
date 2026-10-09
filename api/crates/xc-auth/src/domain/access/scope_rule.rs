use thiserror::Error;
use xc_kernel::ids::BranchId;

use super::role::Role;
use super::scoped_role::ScopedRole;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScopeError {
    #[error("role {0:?} must be organisation-wide")]
    OrganisationWideRequired(Role),
    #[error("role {0:?} requires a branch")]
    BranchRequired(Role),
    #[error("role {0:?} takes only a single branch")]
    SingleBranchOnly(Role),
    #[error("CUSTOMER is not branch-scoped")]
    CustomerNotBranchScoped,
}

pub struct ScopeRule;

impl ScopeRule {
    pub fn check(
        role: Role,
        branch: Option<&BranchId>,
        existing: &[ScopedRole],
    ) -> Result<(), ScopeError> {
        match role {
            Role::Executive => {
                if branch.is_some() {
                    return Err(ScopeError::OrganisationWideRequired(role));
                }
            }
            Role::BranchManager | Role::Teller => {
                if branch.is_none() {
                    return Err(ScopeError::BranchRequired(role));
                }
                if existing.iter().any(|r| r.role == role) {
                    return Err(ScopeError::SingleBranchOnly(role));
                }
            }
            Role::RateManager | Role::Finance => {
                if branch.is_none() {
                    return Err(ScopeError::BranchRequired(role));
                }
            }
            Role::Admin | Role::VerificationReviewer => {
                // Both organisation-wide and branch-scoped allowed, multiple branches allowed.
            }
            Role::Marketing => {
                // Organisation-wide allowed
            }
            Role::Customer => {
                if branch.is_some() {
                    return Err(ScopeError::CustomerNotBranchScoped);
                }
            }
        }
        Ok(())
    }
}
