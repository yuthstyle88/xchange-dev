//! `ScopedRole::covers`: the one place that says whether a role row reaches a branch, so the approval policies do
//! not each hand-roll the match (design `reversal.md:178-179` and `ledger-inventory.md:364`: "organisation-wide or
//! of the branch"; the same rule as the `Authorizer`, `authorizer.rs`).
//!
//! Written before the implementation by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Signature assumed ([required]): `ScopedRole::covers(&self, branch: Option<&str>) -> bool`. A row with no branch
//! (organisation-wide) covers every branch and a request that names none; a row of one branch covers only that
//! branch.

use chrono::{TimeZone, Utc};
use xc_auth::domain::access::{Role, ScopedRole};
use xc_kernel::ids::BranchId;

fn row(branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        Role::Finance,
        branch.map(BranchId::new),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        None,
    )
}

#[test]
fn an_organisation_wide_row_covers_every_branch_and_none() {
    let org = row(None);
    assert!(org.covers(Some("MAIN")));
    assert!(org.covers(Some("B2")));
    assert!(org.covers(None));
}

#[test]
fn a_branch_row_covers_only_its_branch() {
    let main = row(Some("MAIN"));
    assert!(main.covers(Some("MAIN")));
    assert!(!main.covers(Some("B2")));
    assert!(!main.covers(None));
}
