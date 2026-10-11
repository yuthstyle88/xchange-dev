//! The session branch (SB-1 #837; design `docs/design/blueprint/org-roles-branches.md` §14.2): which branches a user
//! may switch to (S), the branch a new session starts on (R5), and the branch a request works in.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::scoped_role::ScopedRole;

/// One branch as the session sees it: its id, its code (`COALESCE(code, id)`) and its name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRef {
    pub id: BranchId,
    pub code: String,
    pub name: String,
}

impl BranchRef {
    pub fn new(id: &str, code: &str, name: &str) -> Self {
        Self {
            id: BranchId::new(id),
            code: code.to_string(),
            name: name.to_string(),
        }
    }
}

/// The session branch rules of §14.2. Pure: the time is passed in.
pub struct SessionBranch;

impl SessionBranch {
    /// S: the branches the user's valid-now rows name; a row naming no branch opens every branch of the
    /// organisation. Each branch once, ordered by `code`, then `id`.
    pub fn switchable(
        roles: &[ScopedRole],
        org_branches: &[BranchRef],
        now: DateTime<Utc>,
    ) -> Vec<BranchRef> {
        let valid: Vec<&ScopedRole> = roles
            .iter()
            .filter(|r| r.valid_from <= now && r.valid_to.is_none_or(|to| now < to))
            .collect();
        let org_wide = valid.iter().any(|r| r.branch.is_none());
        let mut s: Vec<BranchRef> = org_branches
            .iter()
            .filter(|b| org_wide || valid.iter().any(|r| r.branch.as_ref() == Some(&b.id)))
            .cloned()
            .collect();
        s.sort_by(|a, b| {
            a.code
                .cmp(&b.code)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        s.dedup_by(|a, b| a.id == b.id);
        s
    }

    /// R5: the branch a new session starts on. One branch → it; several → the last choice when it is still in S,
    /// else the first by code; none → `None`.
    pub fn initial(s: &[BranchRef], last: Option<&BranchId>) -> Option<BranchId> {
        if let [only] = s {
            return Some(only.id.clone());
        }
        last.filter(|l| s.iter().any(|b| &b.id == *l))
            .cloned()
            .or_else(|| s.first().map(|b| b.id.clone()))
    }

    /// The branch a request works in: the stored session branch while it is still in S, else [`Self::initial`].
    /// Computed on read; the session row is never rewritten by it.
    pub fn effective(
        stored: Option<&BranchId>,
        s: &[BranchRef],
        last: Option<&BranchId>,
    ) -> Option<BranchId> {
        stored
            .filter(|st| s.iter().any(|b| &b.id == *st))
            .cloned()
            .or_else(|| Self::initial(s, last))
    }
}
