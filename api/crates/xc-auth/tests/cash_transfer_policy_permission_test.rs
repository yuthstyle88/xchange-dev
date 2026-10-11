//! Up-front tests for ticket #1272 (CA-0p: the `CashTransferPolicySet` permission,
//! EXECUTIVE only, plus `transferPolicySave`'s area row).
//!
//! Blueprint §3.1, §9 row CA-0p and §9.1 test row.
//! Written before the implementation by gemini/studio-12 (rule A, test-writer,
//! ordered by claude/ad05bf8d). Do not edit, delete or weaken.

mod policy_common;

use chrono::Utc;
use policy_common::*;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_permissions::RolePermissions;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;

#[test]
fn executive_holds_cash_transfer_policy_set_and_no_other_role_does() {
    // 1. EXECUTIVE holds CashTransferPolicySet
    assert!(
        RolePermissions::of(Role::Executive).contains(&Permission::CashTransferPolicySet),
        "EXECUTIVE must hold CashTransferPolicySet"
    );

    // 2. Explicitly verify RATE_MANAGER and ADMIN do not hold CashTransferPolicySet
    assert!(
        !RolePermissions::of(Role::RateManager).contains(&Permission::CashTransferPolicySet),
        "RATE_MANAGER must not hold CashTransferPolicySet"
    );
    assert!(
        !RolePermissions::of(Role::Admin).contains(&Permission::CashTransferPolicySet),
        "ADMIN must not hold CashTransferPolicySet"
    );

    // 3. Enumerate every non-executive role
    let non_executive_roles = [
        Role::Admin,
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ];
    for role in non_executive_roles {
        assert!(
            !RolePermissions::of(role).contains(&Permission::CashTransferPolicySet),
            "{:?} must not hold CashTransferPolicySet",
            role
        );
    }

    // 4. Over the entire Blueprint role set, exactly EXECUTIVE holds it
    let holders: Vec<Role> = BLUEPRINT
        .into_iter()
        .filter(|r| RolePermissions::of(*r).contains(&Permission::CashTransferPolicySet))
        .collect();
    assert_eq!(
        holders,
        vec![Role::Executive],
        "EXECUTIVE alone holds CashTransferPolicySet"
    );
}

#[test]
fn transfer_policy_save_needs_cash_transfer_policy_set() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("transferPolicySave")),
        Some(Need::Permission(Permission::CashTransferPolicySet)),
        "transferPolicySave on Rpc must need CashTransferPolicySet"
    );
}

#[test]
fn policy_authorize_on_transfer_policy_save_grants_executive_and_refuses_all_other_roles() {
    let now = Utc::now();
    let action = Action::new("transferPolicySave");

    // EXECUTIVE is granted
    let exec_actor = staff(Role::Executive, "MAIN");
    let grant = Policy::authorize(Some(&exec_actor), EntryPath::Rpc, &action, None, now);
    assert!(
        grant.is_ok(),
        "EXECUTIVE must be granted transferPolicySave, got: {grant:?}"
    );

    // Explicitly verify RATE_MANAGER and ADMIN are refused with roleNotAllowedHere
    let rm_actor = staff(Role::RateManager, "MAIN");
    assert_eq!(
        Policy::authorize(Some(&rm_actor), EntryPath::Rpc, &action, None, now),
        Err(PolicyError::RoleNotAllowedHere),
        "RATE_MANAGER must receive roleNotAllowedHere"
    );

    let admin_actor = staff(Role::Admin, "MAIN");
    assert_eq!(
        Policy::authorize(Some(&admin_actor), EntryPath::Rpc, &action, None, now),
        Err(PolicyError::RoleNotAllowedHere),
        "ADMIN must receive roleNotAllowedHere"
    );

    // Enumerate every non-executive role: each receives roleNotAllowedHere
    let other_roles = [
        Role::Admin,
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ];
    for role in other_roles {
        let actor = staff(role, "MAIN");
        assert_eq!(
            Policy::authorize(Some(&actor), EntryPath::Rpc, &action, None, now),
            Err(PolicyError::RoleNotAllowedHere),
            "{:?} must receive roleNotAllowedHere on transferPolicySave",
            role
        );
    }
}

#[test]
fn policy_authorize_on_transfer_policy_save_signed_out_gives_sign_in() {
    let now = Utc::now();
    let action = Action::new("transferPolicySave");
    assert_eq!(
        Policy::authorize(None, EntryPath::Rpc, &action, None, now),
        Err(PolicyError::SignIn),
        "signed-out caller must receive signIn"
    );
}
