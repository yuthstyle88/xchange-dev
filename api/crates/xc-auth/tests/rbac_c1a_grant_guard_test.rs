//! RBAC C1a up-front tests (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.1
//! rules 1–7, §3.2 table; owner decisions Q3 A and Q4 A): the `GrantGuard` truth table and
//! `UserStatus::may_move`.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! This file names the new types (`GrantGuard`, `AccountAction`, `GrantRefusal`, `TargetUser`,
//! `UserStatus::Suspended`, `may_move`), so on `origin/main` it fails to compile; that is its red.
//! No database. Roles start at the epoch and never end unless a case says so.

use chrono::{DateTime, Duration, Utc};
use xc_auth::domain::access::scope_rule::ScopeError;
use xc_auth::domain::access::{
    AccountAction, Actor, GrantGuard, GrantRefusal, Role, ScopedRole, TargetUser,
};
use xc_auth::domain::user::UserStatus;
use xc_kernel::ids::BranchId;

const B01: &str = "XC-B01";
const B09: &str = "XC-B09";

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

fn role(r: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(r, branch.map(BranchId::new), epoch(), None)
}

/// A role that ended a day after the epoch.
fn ended(r: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        r,
        branch.map(BranchId::new),
        epoch(),
        Some(epoch() + Duration::days(1)),
    )
}

fn actor(id: &str, roles: Vec<ScopedRole>) -> Actor {
    Actor::new(id, "org", roles, None)
}

fn target(id: &str, status: UserStatus, roles: Vec<ScopedRole>) -> TargetUser {
    TargetUser {
        id: id.to_owned(),
        status,
        roles,
        has_auth_subject: true,
    }
}

fn org_admin() -> Actor {
    actor("a-org", vec![role(Role::Admin, None)])
}

fn branch_admin() -> Actor {
    actor("a-b01", vec![role(Role::Admin, Some(B01))])
}

fn owner() -> Actor {
    actor(
        "owner",
        vec![role(Role::Executive, None), role(Role::Admin, None)],
    )
}

fn active(id: &str, roles: Vec<ScopedRole>) -> TargetUser {
    target(id, UserStatus::Active, roles)
}

fn teller_b01() -> TargetUser {
    active("t-b01", vec![role(Role::Teller, Some(B01))])
}

fn an_owner(id: &str, status: UserStatus) -> TargetUser {
    target(
        id,
        status,
        vec![role(Role::Executive, None), role(Role::Admin, None)],
    )
}

fn check(
    a: &Actor,
    t: &TargetUser,
    action: AccountAction,
    owners_left: usize,
) -> Result<(), GrantRefusal> {
    GrantGuard::check(a, t, &action, owners_left)
}

// ---- rule 1: UsersManage over the target's scope (today's `creator_may_grant`)

#[test]
fn an_org_wide_admin_disables_a_branch_teller() {
    assert_eq!(
        check(&org_admin(), &teller_b01(), AccountAction::Disable, 1),
        Ok(())
    );
}

#[test]
fn a_branch_admin_reaches_only_its_branch() {
    let t09 = active("t-b09", vec![role(Role::Teller, Some(B09))]);
    assert_eq!(
        check(&branch_admin(), &t09, AccountAction::Disable, 1),
        Err(GrantRefusal::NoScope)
    );
    assert_eq!(
        check(&branch_admin(), &teller_b01(), AccountAction::Suspend, 1),
        Ok(())
    );
}

#[test]
fn a_branch_admin_does_not_reach_an_org_wide_role_rule_1_before_rule_3() {
    let exec = active("e", vec![role(Role::Executive, None)]);
    assert_eq!(
        check(&branch_admin(), &exec, AccountAction::Disable, 1),
        Err(GrantRefusal::NoScope)
    );
}

#[test]
fn a_caller_without_a_live_admin_role_has_no_scope() {
    let roleless = active("x", vec![]);
    for who in [
        actor("teller", vec![role(Role::Teller, Some(B01))]),
        actor("exec", vec![role(Role::Executive, None)]),
        actor("expired", vec![ended(Role::Admin, None)]),
    ] {
        assert_eq!(
            check(&who, &teller_b01(), AccountAction::Disable, 1),
            Err(GrantRefusal::NoScope),
            "{} on a teller",
            who.user_id
        );
        assert_eq!(
            check(&who, &roleless, AccountAction::Suspend, 1),
            Err(GrantRefusal::NoScope),
            "{} on a user with no role",
            who.user_id
        );
    }
}

#[test]
fn a_grant_is_scoped_by_the_role_granted() {
    let fin = |b| AccountAction::Grant(role(Role::Finance, Some(b)));
    assert_eq!(check(&branch_admin(), &teller_b01(), fin(B01), 1), Ok(()));
    assert_eq!(
        check(&branch_admin(), &teller_b01(), fin(B09), 1),
        Err(GrantRefusal::NoScope)
    );
    assert_eq!(check(&org_admin(), &teller_b01(), fin(B09), 1), Ok(()));
}

#[test]
fn a_revoke_is_scoped_by_the_role_revoked() {
    let t = active(
        "t",
        vec![
            role(Role::Teller, Some(B01)),
            role(Role::Finance, Some(B09)),
        ],
    );
    let rev = |b| AccountAction::Revoke(role(Role::Finance, Some(b)));
    assert_eq!(
        check(&branch_admin(), &t, rev(B09), 1),
        Err(GrantRefusal::NoScope)
    );
    assert_eq!(check(&org_admin(), &t, rev(B09), 1), Ok(()));
}

// ---- rule 2: no change to one's own account, except one's own profile

#[test]
fn nobody_changes_their_own_account() {
    let me = active("a-org", vec![role(Role::Admin, None)]);
    for action in [
        AccountAction::Disable,
        AccountAction::Suspend,
        AccountAction::ResetPassword,
        AccountAction::Grant(role(Role::Finance, Some(B01))),
        AccountAction::Revoke(role(Role::Admin, None)),
    ] {
        assert_eq!(
            check(&org_admin(), &me, action.clone(), 2),
            Err(GrantRefusal::SelfChange),
            "{action:?}"
        );
    }
    let owner_me = an_owner("owner", UserStatus::Active);
    assert_eq!(
        check(&owner(), &owner_me, AccountAction::Disable, 2),
        Err(GrantRefusal::SelfChange),
        "the owner too"
    );
}

#[test]
fn one_may_edit_one_own_profile() {
    let me = active("a-org", vec![role(Role::Admin, None)]);
    assert_eq!(
        check(&org_admin(), &me, AccountAction::EditProfile, 1),
        Ok(())
    );
}

// ---- rule 3: an ADMIN or EXECUTIVE target is the owner's alone

#[test]
fn an_admin_cannot_change_another_admin_or_an_executive() {
    let other_admin = active("a2", vec![role(Role::Admin, Some(B01))]);
    let exec = active("e", vec![role(Role::Executive, None)]);
    for t in [&other_admin, &exec] {
        for action in [
            AccountAction::Disable,
            AccountAction::Suspend,
            AccountAction::ResetPassword,
            AccountAction::EditProfile,
            AccountAction::Grant(role(Role::Finance, Some(B01))),
        ] {
            assert_eq!(
                check(&org_admin(), t, action.clone(), 1),
                Err(GrantRefusal::ProtectedTarget),
                "{} {action:?}",
                t.id
            );
        }
    }
}

#[test]
fn the_owner_changes_an_admin_or_an_executive() {
    let other_admin = active("a2", vec![role(Role::Admin, Some(B01))]);
    let exec = active("e", vec![role(Role::Executive, None)]);
    assert_eq!(
        check(&owner(), &other_admin, AccountAction::Suspend, 1),
        Ok(())
    );
    assert_eq!(check(&owner(), &exec, AccountAction::Disable, 1), Ok(()));
}

#[test]
fn an_ended_admin_role_does_not_protect_its_former_holder() {
    let former = active(
        "f",
        vec![ended(Role::Admin, Some(B01)), role(Role::Teller, Some(B01))],
    );
    assert_eq!(
        check(&org_admin(), &former, AccountAction::Disable, 1),
        Ok(())
    );
}

// ---- rule 4: only the owner grants ADMIN or EXECUTIVE

#[test]
fn an_admin_cannot_grant_admin_or_executive() {
    for granted in [
        role(Role::Admin, Some(B01)),
        role(Role::Admin, None),
        role(Role::Executive, None),
    ] {
        assert_eq!(
            check(
                &org_admin(),
                &teller_b01(),
                AccountAction::Grant(granted.clone()),
                1
            ),
            Err(GrantRefusal::ProtectedRole),
            "{granted:?}"
        );
    }
}

#[test]
fn executive_with_a_branch_admin_is_not_the_owner() {
    let half = actor(
        "half",
        vec![role(Role::Executive, None), role(Role::Admin, Some(B01))],
    );
    assert_eq!(
        check(
            &half,
            &teller_b01(),
            AccountAction::Grant(role(Role::Admin, Some(B01))),
            1
        ),
        Err(GrantRefusal::ProtectedRole)
    );
}

#[test]
fn the_owner_grants_admin_and_executive() {
    assert_eq!(
        check(
            &owner(),
            &teller_b01(),
            AccountAction::Grant(role(Role::Admin, None)),
            1
        ),
        Ok(())
    );
    assert_eq!(
        check(
            &owner(),
            &teller_b01(),
            AccountAction::Grant(role(Role::Executive, None)),
            1
        ),
        Ok(())
    );
}

// ---- rule 5: only an EXECUTIVE re-enables a DISABLED user

#[test]
fn enabling_a_disabled_user_needs_executive() {
    let gone = target(
        "g",
        UserStatus::Disabled,
        vec![role(Role::Teller, Some(B01))],
    );
    assert_eq!(
        check(&org_admin(), &gone, AccountAction::Enable, 1),
        Err(GrantRefusal::EnableNeedsExecutive)
    );
    assert_eq!(check(&owner(), &gone, AccountAction::Enable, 1), Ok(()));
}

#[test]
fn an_admin_suspends_and_resumes() {
    let held = target(
        "h",
        UserStatus::Suspended,
        vec![role(Role::Teller, Some(B01))],
    );
    assert_eq!(
        check(&org_admin(), &teller_b01(), AccountAction::Suspend, 1),
        Ok(())
    );
    assert_eq!(check(&org_admin(), &held, AccountAction::Resume, 1), Ok(()));
}

// ---- rule 6: the last ACTIVE owner stays an ACTIVE owner

#[test]
fn the_last_owner_cannot_be_suspended_disabled_or_stripped() {
    let last = an_owner("o2", UserStatus::Active);
    for action in [
        AccountAction::Suspend,
        AccountAction::Disable,
        AccountAction::Revoke(role(Role::Executive, None)),
        AccountAction::Revoke(role(Role::Admin, None)),
    ] {
        assert_eq!(
            check(&owner(), &last, action.clone(), 1),
            Err(GrantRefusal::LastOwner),
            "{action:?}"
        );
        assert_eq!(
            check(&owner(), &last, action.clone(), 2),
            Ok(()),
            "{action:?} with two owners"
        );
    }
}

#[test]
fn a_change_that_keeps_the_owner_an_owner_is_not_the_last_owner_rule() {
    let last = target(
        "o2",
        UserStatus::Active,
        vec![
            role(Role::Executive, None),
            role(Role::Admin, None),
            role(Role::Finance, Some(B01)),
        ],
    );
    assert_eq!(
        check(
            &owner(),
            &last,
            AccountAction::Revoke(role(Role::Finance, Some(B01))),
            1
        ),
        Ok(())
    );
    assert_eq!(
        check(&owner(), &last, AccountAction::ResetPassword, 1),
        Ok(())
    );
}

// ---- rule 7: ScopeRule, unchanged, after the rules above

#[test]
fn a_grant_that_breaks_the_scope_rule_is_a_scope_refusal() {
    assert_eq!(
        check(
            &org_admin(),
            &teller_b01(),
            AccountAction::Grant(role(Role::Teller, Some(B09))),
            1
        ),
        Err(GrantRefusal::Scope(ScopeError::SingleBranchOnly(
            Role::Teller
        )))
    );
    assert_eq!(
        check(
            &org_admin(),
            &teller_b01(),
            AccountAction::Grant(role(Role::Finance, None)),
            1
        ),
        Err(GrantRefusal::Scope(ScopeError::BranchRequired(
            Role::Finance
        )))
    );
}

// ---- §3.2: the status moves

#[test]
fn may_move_is_exactly_the_design_table() {
    use UserStatus::*;
    let all = [Pending, Active, Suspended, Disabled];
    let allowed = [
        (Pending, Active),
        (Pending, Disabled),
        (Active, Suspended),
        (Suspended, Active),
        (Active, Disabled),
        (Suspended, Disabled),
        (Disabled, Active),
    ];
    for from in all {
        for to in all {
            assert_eq!(
                from.may_move(to),
                allowed.contains(&(from, to)),
                "{from} -> {to}"
            );
        }
    }
}
