//! `BranchScope` (EX-1 #1070; `docs/design/blueprint/executive-finance.md` §7.2): which branches an actor may
//! read for a permission. A01 reads every branch for an organisation-wide EXECUTIVE; F01 reads exactly a FINANCE's
//! branches (owner decision E-7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::actor::Actor;
use super::authorizer::qualifying;
use super::permission::Permission;

/// The branches of the actor's own organisation that a permission reaches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchScope {
    /// An organisation-wide role holds the permission: every branch.
    All,
    /// Exactly these branches, sorted, with no duplicates. Empty when no role holds the permission now.
    Branches(Vec<BranchId>),
}

impl BranchScope {
    /// From the roles that qualify for `permission` at `now` (the same rule as `Authorizer::check`):
    /// one organisation-wide row gives `All`, otherwise the rows' branches.
    pub fn of(actor: &Actor, permission: Permission, now: DateTime<Utc>) -> BranchScope {
        let mut branches = Vec::new();
        for role in qualifying(actor, permission, now) {
            match &role.branch {
                None => return BranchScope::All,
                Some(b) => branches.push(b.clone()),
            }
        }
        branches.sort();
        branches.dedup();
        BranchScope::Branches(branches)
    }

    /// Whether `branch` is in scope.
    pub fn contains(&self, branch: &BranchId) -> bool {
        match self {
            BranchScope::All => true,
            BranchScope::Branches(list) => list.contains(branch),
        }
    }
}
