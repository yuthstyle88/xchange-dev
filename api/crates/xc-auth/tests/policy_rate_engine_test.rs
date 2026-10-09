//! Ticket #598 (RE-1+RE-3) — the policy rows RE-3 adds for the rate engine's actions.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §6 "Policy rows" (the only edit to the role table, in
//! RE-3):
//!
//! | Action | Rpc (owner page) | StaffRpc |
//! |---|---|---|
//! | `ratePolicySave` | OWNER | — |
//! | `rateDraft` (save, submit, simulate, publish, fetch reference) | OWNER, RATES | RATES |
//! | `approvalDecide` | OWNER | — |
//! | `approvalRead` | OWNER | — |
//!
//! and §9's table of provided tests that change: `policy_test.rs:386,705` (`adminSaveRate ==
//! ["RATES"]`) stays as it is in RE-3 — `adminSaveRate` is removed only in RE-7. Nothing in
//! `policy_test.rs` or its fixture changes: these are new action names v2 never had.
//!
//! Signatures: none new; the existing `Policy::allow(actor, path, action)`.
//! Refusals the design does not name, chosen here to follow the table's existing rules: on Rpc, a
//! staff role outside a row is `RoleNotAllowedHere` (as `adminSaveRate`, `policy.rs:76-92`); on
//! StaffRpc, a role with no row for the action is `RoleNotAllowed` (`policy.rs:114-126`), and the
//! owner is `EmployeeAuthRequired`; nobody signed in is `SignIn` on Rpc and
//! `EmployeeAuthRequired` on StaffRpc.

mod policy_common;

use policy_common::*;

const B2: &str = "B2";

fn allowed_roles(on: EntryPath, name: &str) -> Vec<&'static str> {
    Role::STAFF
        .into_iter()
        .filter(|r| ask(Some(&staff(*r, B2)), on, name).is_ok())
        .map(Role::as_str)
        .collect()
}

#[test]
fn rate_draft_is_owner_and_rates_on_rpc_and_rates_on_the_staff_bridge() {
    assert_eq!(ask(Some(&owner()), EntryPath::Rpc, "rateDraft"), Ok(()));
    assert_eq!(allowed_roles(EntryPath::Rpc, "rateDraft"), ["RATES"]);
    assert_eq!(allowed_roles(EntryPath::StaffRpc, "rateDraft"), ["RATES"]);
    assert_eq!(
        ask(Some(&owner()), EntryPath::StaffRpc, "rateDraft"),
        Err(PolicyError::EmployeeAuthRequired)
    );
    for r in Role::STAFF.into_iter().filter(|r| *r != Role::Rates) {
        assert_eq!(
            ask(Some(&staff(r, B2)), EntryPath::Rpc, "rateDraft"),
            Err(PolicyError::RoleNotAllowedHere),
            "{} on Rpc",
            r.as_str()
        );
        assert_eq!(
            ask(Some(&staff(r, B2)), EntryPath::StaffRpc, "rateDraft"),
            Err(PolicyError::RoleNotAllowed),
            "{} on StaffRpc",
            r.as_str()
        );
    }
    assert_eq!(
        ask(None, EntryPath::Rpc, "rateDraft"),
        Err(PolicyError::SignIn)
    );
    assert_eq!(
        ask(None, EntryPath::StaffRpc, "rateDraft"),
        Err(PolicyError::EmployeeAuthRequired)
    );
}

#[test]
fn rate_policy_save_approval_decide_and_approval_read_are_the_owners_alone() {
    for name in ["ratePolicySave", "approvalDecide", "approvalRead"] {
        assert_eq!(ask(Some(&owner()), EntryPath::Rpc, name), Ok(()), "{name}");
        assert!(
            allowed_roles(EntryPath::Rpc, name).is_empty(),
            "{name}: no staff role on Rpc"
        );
        assert!(
            allowed_roles(EntryPath::StaffRpc, name).is_empty(),
            "{name}: no staff role on StaffRpc"
        );
        for r in Role::STAFF {
            assert_eq!(
                ask(Some(&staff(r, B2)), EntryPath::Rpc, name),
                Err(PolicyError::RoleNotAllowedHere),
                "{name}: {} on Rpc",
                r.as_str()
            );
            assert_eq!(
                ask(Some(&staff(r, B2)), EntryPath::StaffRpc, name),
                Err(PolicyError::RoleNotAllowed),
                "{name}: {} on StaffRpc",
                r.as_str()
            );
        }
        assert_eq!(
            ask(None, EntryPath::Rpc, name),
            Err(PolicyError::SignIn),
            "{name}"
        );
    }
}

#[test]
fn the_new_rows_leave_the_existing_rows_as_they_were() {
    // RE-3 adds rows; the direct save stays RATES-only until RE-7, and the snapshot is everyone's
    for on in [EntryPath::Rpc, EntryPath::StaffRpc] {
        assert_eq!(allowed_roles(on, "adminSaveRate"), ["RATES"]);
        assert_eq!(allowed_roles(on, "adminSnapshot").len(), 9);
    }
    assert_eq!(ask(Some(&owner()), EntryPath::Rpc, "adminSaveRate"), Ok(()));
    // an action no row names is still refused, on both paths
    assert_eq!(
        ask(Some(&owner()), EntryPath::Rpc, "rateDraftt"),
        Err(PolicyError::InvalidRequest)
    );
    assert_eq!(
        ask(
            Some(&staff(Role::Rates, B2)),
            EntryPath::StaffRpc,
            "rateDraftt"
        ),
        Err(PolicyError::RoleNotAllowed)
    );
}
