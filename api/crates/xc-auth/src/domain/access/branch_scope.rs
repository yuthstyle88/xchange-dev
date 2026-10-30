//! `BranchScope` (EX-1 #1070; `docs/design/blueprint/executive-finance.md` §7.2): which branches an actor may
//! read for a permission. A01 reads every branch for an organisation-wide EXECUTIVE; F01 reads exactly a FINANCE's
//! branches (owner decision E-7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::actor::Actor;
use super::authorizer::qualifying;
use super::permission::Permission;
use super::scoped_role::ScopedRole;

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
        Self::of_rows(qualifying(actor, permission, now))
    }

    /// From rows the caller already chose (`GrantGuard`: the actor's live ADMIN rows): one organisation-wide row
    /// gives `All`, otherwise the rows' branches (#5557).
    pub fn of_rows<'a>(rows: impl IntoIterator<Item = &'a ScopedRole>) -> BranchScope {
        let mut branches = Vec::new();
        for role in rows {
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

    /// Whether a row at `branch` (`None` = organisation-wide) is in scope: `All` covers every row, `Branches` only a
    /// row on one of its branches (#5557; `GrantGuard`'s `covers`).
    pub fn covers_row(&self, branch: Option<&BranchId>) -> bool {
        match self {
            BranchScope::All => true,
            BranchScope::Branches(list) => branch.is_some_and(|b| list.contains(b)),
        }
    }

    /// Whether a user whose live rows sit at `branches` is in scope (#5557): `All` covers every user; `Branches`
    /// covers a user with at least one live row, every one of them on a covered branch. A user with no live row is
    /// covered by `All` only.
    pub fn covers_user<'a>(
        &self,
        branches: impl IntoIterator<Item = Option<&'a BranchId>>,
    ) -> bool {
        match self {
            BranchScope::All => true,
            BranchScope::Branches(_) => {
                let mut rows = branches.into_iter().peekable();
                rows.peek().is_some() && rows.all(|b| self.covers_row(b))
            }
        }
    }
}
