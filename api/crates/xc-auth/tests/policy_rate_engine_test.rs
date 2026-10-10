//! Ticket #598 (RE-1+RE-3) — the policy rows RE-3 adds for the rate engine's actions.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Provided-test amendment for #602 (RE-5), by the same author: `ratePolicySave` is OWNER and RATES
//! on Rpc (lane controller's ruling 2026-10-09: RATE_MANAGER may save a policy version changing
//! only buy/sell %, per Blueprint RATE_MANAGER 'ปรับ %'). RATES may move only the percents; that
//! rule is the command's (`api/tests/v3_rate_policy_save_test.rs`), the row here only lets RATES in.
//! StaffRpc has no row for it, as before.
//!
//! Design: `docs/design/blueprint/rates.md` §6 "Policy rows" (the only edit to the role table, in
//! RE-3):
//!
//! | Action | Rpc (owner page) | StaffRpc |
//! |---|---|---|
//! | `ratePolicySave` | OWNER, RATES (#602) | — |
//! | `rateDraft` (save, submit, simulate, publish, fetch reference) | OWNER, RATES | RATES |
//! | `approvalDecide` | OWNER, MANAGER/BACKEND, FINANCE (#874) | — |
//! | `approvalRead` | OWNER, MANAGER/BACKEND, FINANCE (#874) | — |
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
//!
//! Moved by the author of OR-4 (#550, `docs/design/blueprint/org-roles-branches.md` §3.4, §4.2,
//! §4.3) onto the Blueprint's users. The rows become permissions, roles by B1 (OWNER → EXECUTIVE,
//! RATES → RATE_MANAGER): `rateDraft` and `adminSaveRate` are RatesDraft (RATE_MANAGER, EXECUTIVE);
//! `ratePolicySave` is RatePolicySave, which the role table therefore gives RATE_MANAGER beside
//! EXECUTIVE (the lane controller's ruling of 2026-10-09 above, #602); `approvalDecide` and
//! `approvalRead` are ApprovalsDecide, EXECUTIVE's (the B1 translation of "OWNER" alone). One
//! answer changes, C1: on StaffRpc the owner's user is answered by its roles, so it may `rateDraft`
//! there through EXECUTIVE (was `EmployeeAuthRequired`).

mod policy_common;

use policy_common::*;

const B2: &str = "B2";

fn allowed_roles(on: EntryPath, name: &str) -> Vec<&'static str> {
    BLUEPRINT
        .into_iter()
        .filter(|r| ask(Some(&staff(*r, B2)), on, name).is_ok())
        .map(Role::as_str)
        .collect()
}

#[test]
fn rate_draft_is_owner_and_rates_on_rpc_and_rates_on_the_staff_bridge() {
    assert_eq!(ask(Some(&owner()), EntryPath::Rpc, "rateDraft"), Ok(()));
    assert_eq!(
        allowed_roles(EntryPath::Rpc, "rateDraft"),
        ["EXECUTIVE", "RATE_MANAGER"]
    );
    assert_eq!(
        allowed_roles(EntryPath::StaffRpc, "rateDraft"),
        ["EXECUTIVE", "RATE_MANAGER"]
    );
    // C1: was EmployeeAuthRequired
    assert_eq!(
        ask(Some(&owner()), EntryPath::StaffRpc, "rateDraft"),
        Ok(())
    );
    for r in BLUEPRINT
        .into_iter()
        .filter(|r| !matches!(r, Role::Executive | Role::RateManager))
    {
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
fn rate_policy_save_is_owner_and_rates_on_rpc_and_no_row_on_the_staff_bridge() {
    // #602: RATE_MANAGER "ปรับ %" (Blueprint §02); the command keeps RATE_MANAGER to the percents
    assert_eq!(
        ask(Some(&owner()), EntryPath::Rpc, "ratePolicySave"),
        Ok(())
    );
    assert_eq!(
        allowed_roles(EntryPath::Rpc, "ratePolicySave"),
        ["EXECUTIVE", "RATE_MANAGER"]
    );
    assert!(allowed_roles(EntryPath::StaffRpc, "ratePolicySave").is_empty());
    for r in BLUEPRINT
        .into_iter()
        .filter(|r| !matches!(r, Role::Executive | Role::RateManager))
    {
        assert_eq!(
            ask(Some(&staff(r, B2)), EntryPath::Rpc, "ratePolicySave"),
            Err(PolicyError::RoleNotAllowedHere),
            "{} on Rpc",
            r.as_str()
        );
    }
    for r in BLUEPRINT {
        assert_eq!(
            ask(Some(&staff(r, B2)), EntryPath::StaffRpc, "ratePolicySave"),
            Err(PolicyError::RoleNotAllowed),
            "{} on StaffRpc",
            r.as_str()
        );
    }
    assert_eq!(
        ask(None, EntryPath::Rpc, "ratePolicySave"),
        Err(PolicyError::SignIn)
    );
}

/// Amended for #874 by its ticket author (claude/f03345fe), per lane controller (2026-10-10):
/// BRANCH_MANAGER and (RG4 of #682) FINANCE qualify for ApprovalsDecide; each type's policy still decides (rates stay the
/// owner's through `RatePublicationPolicy`). Every other assertion is unchanged.
#[test]
fn approval_decide_and_approval_read_are_the_executives_the_branch_managers_and_finances() {
    for name in ["approvalDecide", "approvalRead"] {
        assert_eq!(ask(Some(&owner()), EntryPath::Rpc, name), Ok(()), "{name}");
        assert_eq!(
            allowed_roles(EntryPath::Rpc, name),
            ["EXECUTIVE", "BRANCH_MANAGER", "FINANCE"],
            "{name}: EXECUTIVE, BRANCH_MANAGER and FINANCE on Rpc"
        );
        assert!(
            allowed_roles(EntryPath::StaffRpc, name).is_empty(),
            "{name}: no role on StaffRpc"
        );
        for r in BLUEPRINT
            .into_iter()
            .filter(|r| ![Role::Executive, Role::BranchManager, Role::Finance].contains(r))
        {
            assert_eq!(
                ask(Some(&staff(r, B2)), EntryPath::Rpc, name),
                Err(PolicyError::RoleNotAllowedHere),
                "{name}: {} on Rpc",
                r.as_str()
            );
        }
        for r in BLUEPRINT {
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
    // RE-3 adds rows; the direct save stays RatesDraft until RE-7, and the snapshot is CompatRead
    for on in [EntryPath::Rpc, EntryPath::StaffRpc] {
        assert_eq!(
            allowed_roles(on, "adminSaveRate"),
            ["EXECUTIVE", "RATE_MANAGER"]
        );
        assert_eq!(allowed_roles(on, "adminSnapshot").len(), 7);
    }
    assert_eq!(ask(Some(&owner()), EntryPath::Rpc, "adminSaveRate"), Ok(()));
    // an action no row names is still refused, on both paths
    assert_eq!(
        ask(Some(&owner()), EntryPath::Rpc, "rateDraftt"),
        Err(PolicyError::InvalidRequest)
    );
    assert_eq!(
        ask(
            Some(&staff(Role::RateManager, B2)),
            EntryPath::StaffRpc,
            "rateDraftt"
        ),
        Err(PolicyError::RoleNotAllowed)
    );
}
