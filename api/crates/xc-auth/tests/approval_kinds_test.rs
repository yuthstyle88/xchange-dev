//! Up-front tests for RBAC §11.9 (design `docs/design/blueprint/user-management-rbac.md`): the nine approval-kind
//! permissions, their seed (AP-0) and guardrail 6 (AP-2).
//! Written before the implementation by the BE lane controller (AP-0 skeleton, `feat/approval-perms-skeleton`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! No database, no global state: `RoleGrants::seed` and `MatrixGuardrails::check` are pure.
//! - The `seed_*` tests pass at AP-0: they lock §11.9's seed ("seeded so that ship day changes nothing").
//! - The `guardrail_*` tests fail until AP-2 (rule 5 no longer covers the nine nor `ApprovalsDecide`; new rule 6).

use std::collections::BTreeSet;

use xc_auth::domain::access::permission_module::{APPROVAL_KINDS, PermissionModule};
use xc_auth::domain::access::{
    Group, GuardrailBreach, MatrixGuardrails, Permission, Role, RoleGrants,
};

use Permission as P;

/// §11.9's table: each kind and the one built-in group it is seeded on.
const SEEDED_ON: [(Permission, Role); 9] = [
    (P::ApproveCashTransferWithinLimit, Role::BranchManager),
    (P::ApproveCashTransferAboveLimit, Role::Executive),
    (P::ApproveCountDifference, Role::Finance),
    (P::ApproveNegativeStock, Role::BranchManager),
    (P::ApproveCampaignVersion, Role::Executive),
    (P::ApproveRatePublication, Role::Executive),
    (P::ApproveExchangeTransaction, Role::BranchManager),
    (P::ApproveTransactionReversal, Role::Finance),
    (P::ApproveLoyaltyRuleVersion, Role::Executive),
];

#[test]
fn seed_approval_kinds_are_the_nine_in_permission_all_with_no_module() {
    let kinds: BTreeSet<Permission> = APPROVAL_KINDS.into_iter().collect();
    assert_eq!(kinds.len(), 9, "APPROVAL_KINDS has no duplicates");
    assert_eq!(
        kinds,
        SEEDED_ON.iter().map(|&(p, _)| p).collect::<BTreeSet<_>>(),
        "APPROVAL_KINDS is §11.9's nine"
    );
    assert_eq!(Permission::ALL.len(), 61, "Permission::ALL grows 52 -> 61");
    for p in APPROVAL_KINDS {
        assert_eq!(
            Permission::ALL.iter().filter(|q| **q == p).count(),
            1,
            "{p:?} appears once in Permission::ALL"
        );
        assert_eq!(PermissionModule::module(p), None, "{p:?} is no-module");
    }
}

/// §11.9: each kind is seeded on exactly the base role that decides it today, and on no other built-in group.
#[test]
fn seed_each_kind_is_held_by_exactly_its_seeded_role() {
    for (p, want) in SEEDED_ON {
        let holders: Vec<Role> = RoleGrants::BUILT_IN
            .into_iter()
            .filter(|&r| RoleGrants::seed(r).contains(&p))
            .collect();
        assert_eq!(holders, vec![want], "{p:?} is seeded on {want:?} only");
    }
}

/// §11.9 "`ApprovalsDecide` follows the kinds": in the seed, a built-in group holds it iff it holds one of the nine.
#[test]
fn seed_approvals_decide_iff_some_kind() {
    for r in RoleGrants::BUILT_IN {
        let seed = RoleGrants::seed(r);
        let any_kind = APPROVAL_KINDS.iter().any(|p| seed.contains(p));
        assert_eq!(
            seed.contains(&P::ApprovalsDecide),
            any_kind,
            "{}: ApprovalsDecide iff some approval kind",
            r.as_str()
        );
    }
}

/// §11.9 data migration: `SEED_VERSION` 3 -> 4, and each kind is in `SEED_ADDITIONS` at version 4, exactly once.
#[test]
fn seed_version_4_adds_the_nine() {
    assert_eq!(RoleGrants::SEED_VERSION, 4);
    for p in APPROVAL_KINDS {
        let at: Vec<i32> = RoleGrants::SEED_ADDITIONS
            .iter()
            .filter(|&&(_, q)| q == p)
            .map(|&(v, _)| v)
            .collect();
        assert_eq!(
            at,
            vec![4],
            "{p:?} enters SEED_ADDITIONS once, at version 4"
        );
    }
    assert!(
        RoleGrants::SEED_REMOVALS
            .iter()
            .all(|&(_, p)| !APPROVAL_KINDS.contains(&p)),
        "§11.9: nothing is removed"
    );
}

/// Rule 6's number is the `n` of F12's `409 guardrail:<n>`.
#[test]
fn guardrail_6_has_number_6() {
    assert_eq!(GuardrailBreach::AdminBaseHoldsApproval.number(), 6);
}

fn custom(code: &str, base: Role) -> Group {
    Group {
        code: code.to_owned(),
        name: code.to_owned(),
        base,
        built_in: false,
    }
}

/// `base`'s seed plus `add`, with `ApprovalsDecide` held iff some kind is (the set the matrix save would build).
fn with_kinds(base: Role, add: &[Permission]) -> Vec<Permission> {
    let mut set = RoleGrants::seed(base);
    for p in add {
        if !set.contains(p) {
            set.push(*p);
        }
    }
    if set.iter().any(|p| APPROVAL_KINDS.contains(p)) && !set.contains(&P::ApprovalsDecide) {
        set.push(P::ApprovalsDecide);
    }
    set
}

/// §11.9 A7: a group whose base is ADMIN holding `ApproveNegativeStock` is refused with guardrail 6, not 5.
#[test]
fn guardrail_6_a7_an_admin_based_group_holding_a_kind_is_refused() {
    assert_eq!(
        MatrixGuardrails::check(
            &custom("ADMIN_PLUS", Role::Admin),
            &with_kinds(Role::Admin, &[P::ApproveNegativeStock]),
        ),
        Err(GuardrailBreach::AdminBaseHoldsApproval)
    );
}

/// Rule 6 holds for every one of the nine, and for the built-in ADMIN group too.
#[test]
fn guardrail_6_holds_for_every_kind_and_the_built_in_admin_group() {
    for p in APPROVAL_KINDS {
        for group in [
            custom("ADMIN_PLUS", Role::Admin),
            Group::built_in(Role::Admin),
        ] {
            assert_eq!(
                MatrixGuardrails::check(&group, &with_kinds(Role::Admin, &[p])),
                Err(GuardrailBreach::AdminBaseHoldsApproval),
                "{} holding {p:?}",
                group.code
            );
        }
    }
}

/// §11.9 A2 (save side): rule 5 no longer covers the nine nor `ApprovalsDecide`, so a group based on TELLER may be
/// granted one. Today rule 5 refuses it (`NoModuleEdited`).
#[test]
fn guardrail_5_a2_a_teller_based_group_may_hold_a_kind() {
    assert_eq!(
        MatrixGuardrails::check(
            &custom("NIGHT_FIN", Role::Teller),
            &with_kinds(Role::Teller, &[P::ApproveTransactionReversal]),
        ),
        Ok(())
    );
}

/// §11.9 A3 (save side): built-in FINANCE may drop one of its kinds; the other no-module permissions still equal
/// its base's seed.
#[test]
fn guardrail_5_a3_finance_may_drop_a_kind() {
    let set: Vec<Permission> = RoleGrants::seed(Role::Finance)
        .into_iter()
        .filter(|p| *p != P::ApproveCountDifference)
        .collect();
    assert_eq!(
        MatrixGuardrails::check(&Group::built_in(Role::Finance), &set),
        Ok(())
    );
}

/// Rule 5 still holds for every other no-module permission: dropping `ShiftOperate` from TELLER is refused.
#[test]
fn guardrail_5_still_covers_the_other_no_module_permissions() {
    let set: Vec<Permission> = with_kinds(Role::Teller, &[P::ApproveTransactionReversal])
        .into_iter()
        .filter(|p| *p != P::ShiftOperate)
        .collect();
    assert_eq!(
        MatrixGuardrails::check(&custom("NIGHT_FIN", Role::Teller), &set),
        Err(GuardrailBreach::NoModuleEdited)
    );
}
