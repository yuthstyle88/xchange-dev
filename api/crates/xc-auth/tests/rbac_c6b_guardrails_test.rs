//! Up-front tests for RBAC §11 ticket C6b (design `docs/design/blueprint/user-management-rbac.md` §11.3):
//! `MatrixGuardrails::check`. Written before the implementation by the sub-controller claude/65357afb on
//! `feat/rbac-groups`. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! No database, no global state. A "no-module" permission is one `PermissionModule::module` maps to `None`; rule 5
//! compares a set's no-module part with the no-module part of `RoleGrants::seed(group.base)`.

use xc_auth::domain::access::permission_module::{Level, Module, PermissionModule};
use xc_auth::domain::access::{
    Group, GuardrailBreach, MatrixGuardrails, Permission, Role, RoleGrants,
};

use Permission as P;

fn custom(code: &str, base: Role) -> Group {
    Group {
        code: code.to_owned(),
        name: code.to_owned(),
        base,
        built_in: false,
    }
}

fn no_module_of(base: Role) -> Vec<Permission> {
    RoleGrants::seed(base)
        .into_iter()
        .filter(|p| PermissionModule::module(*p).is_none())
        .collect()
}

/// `base`'s no-module seed plus `extra`.
fn set(base: Role, extra: &[Permission]) -> Vec<Permission> {
    let mut s = no_module_of(base);
    s.extend_from_slice(extra);
    s
}

fn module_permissions(module: Module, above: Level) -> Vec<Permission> {
    Permission::ALL
        .into_iter()
        .filter(
            |p| matches!(PermissionModule::module(*p), Some((m, l)) if m == module && l > above),
        )
        .collect()
}

#[test]
fn breach_numbers_are_the_designs() {
    assert_eq!(GuardrailBreach::AuditLogsAboveView.number(), 1);
    assert_eq!(GuardrailBreach::AdminModulesAboveView.number(), 2);
    assert_eq!(GuardrailBreach::AdminBaseAboveView.number(), 3);
    assert_eq!(GuardrailBreach::ExecutiveFixed.number(), 4);
    assert_eq!(GuardrailBreach::NoModuleEdited.number(), 5);
}

#[test]
fn every_built_in_seed_except_executive_passes() {
    for role in RoleGrants::BUILT_IN {
        if role == Role::Executive {
            continue;
        }
        assert_eq!(
            MatrixGuardrails::check(&Group::built_in(role), &RoleGrants::seed(role)),
            Ok(()),
            "{}'s seed passes the guardrails",
            role.as_str()
        );
    }
}

/// Rule 4: the built-in EXECUTIVE group is fixed, whatever the set, even its own seed.
#[test]
fn rule_4_the_executive_group_is_fixed() {
    let exec = Group::built_in(Role::Executive);
    assert_eq!(
        MatrixGuardrails::check(&exec, &RoleGrants::seed(Role::Executive)),
        Err(GuardrailBreach::ExecutiveFixed)
    );
    assert_eq!(
        MatrixGuardrails::check(&exec, &[]),
        Err(GuardrailBreach::ExecutiveFixed)
    );
}

/// Rule 4 is answered before any other.
#[test]
fn rule_4_comes_first() {
    let exec = Group::built_in(Role::Executive);
    assert_eq!(
        MatrixGuardrails::check(&exec, &[P::CustomerSelf]),
        Err(GuardrailBreach::ExecutiveFixed)
    );
}

/// A custom group based on EXECUTIVE is not the fixed group: it may hold the admin modules above VIEW.
#[test]
fn a_custom_group_based_on_executive_is_not_fixed() {
    let deputy = custom("DEPUTY", Role::Executive);
    assert_eq!(
        MatrixGuardrails::check(
            &deputy,
            &set(Role::Executive, &[P::UsersManage, P::SystemSettings])
        ),
        Ok(())
    );
}

/// Rule 2: USER_MANAGEMENT and SYSTEM_SETTINGS above VIEW only for an EXECUTIVE or ADMIN base.
#[test]
fn rule_2_admin_modules_above_view_need_an_executive_or_admin_base() {
    let above: Vec<Permission> = module_permissions(Module::UserManagement, Level::View)
        .into_iter()
        .chain(module_permissions(Module::SystemSettings, Level::View))
        .collect();
    assert!(!above.is_empty());
    for base in [
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::VerificationReviewer,
    ] {
        for p in &above {
            assert_eq!(
                MatrixGuardrails::check(&custom("G", base), &set(base, &[*p])),
                Err(GuardrailBreach::AdminModulesAboveView),
                "{p:?} on a {} base",
                base.as_str()
            );
            assert_eq!(
                MatrixGuardrails::check(&Group::built_in(base), &set(base, &[*p])),
                Err(GuardrailBreach::AdminModulesAboveView),
                "{p:?} on built-in {}",
                base.as_str()
            );
        }
    }
    for p in &above {
        assert_eq!(
            MatrixGuardrails::check(&custom("G", Role::Admin), &set(Role::Admin, &[*p])),
            Ok(())
        );
    }
}

/// Rule 3: an ADMIN base holds at most VIEW outside USER_MANAGEMENT and SYSTEM_SETTINGS.
#[test]
fn rule_3_an_admin_base_is_view_only_outside_its_modules() {
    let admin = custom("IT", Role::Admin);
    for p in Permission::ALL {
        let Some((module, level)) = PermissionModule::module(p) else {
            continue;
        };
        if matches!(module, Module::UserManagement | Module::SystemSettings) {
            continue;
        }
        let want = if level > Level::View {
            Err(GuardrailBreach::AdminBaseAboveView)
        } else {
            Ok(())
        };
        assert_eq!(
            MatrixGuardrails::check(&admin, &set(Role::Admin, &[p])),
            want,
            "{p:?}"
        );
    }
    // §11.2 "Branch cash basics": CashManage is now a no-module basic (rule 5), so test rule 3 with FinanceReconcile (FINANCE FULL).
    assert_eq!(
        MatrixGuardrails::check(
            &Group::built_in(Role::Admin),
            &set(Role::Admin, &[P::FinanceReconcile])
        ),
        Err(GuardrailBreach::AdminBaseAboveView),
        "the built-in ADMIN group too"
    );
}

/// Rule 5: the no-module part of the set is its base's seed, no more and no less.
#[test]
fn rule_5_no_module_permissions_are_not_edited() {
    let night = custom("NIGHT_TELLER", Role::Teller);
    assert_eq!(
        MatrixGuardrails::check(&night, &set(Role::Teller, &[P::ExchangeTransact])),
        Ok(())
    );

    let mut dropped = set(Role::Teller, &[P::ExchangeTransact]);
    dropped.retain(|p| *p != P::ShiftOperate);
    assert_eq!(
        MatrixGuardrails::check(&night, &dropped),
        Err(GuardrailBreach::NoModuleEdited),
        "dropping a desk basic"
    );

    let added = set(Role::Teller, &[P::ExchangeTransact, P::RequestAbandon]);
    assert_eq!(
        MatrixGuardrails::check(&night, &added),
        Err(GuardrailBreach::NoModuleEdited),
        "adding a no-module permission the base's seed lacks"
    );
    assert_eq!(
        MatrixGuardrails::check(&Group::built_in(Role::Customer), &[P::SignedIn]),
        Err(GuardrailBreach::NoModuleEdited),
        "CUSTOMER without CustomerSelf"
    );
}

/// Rule 1: AUDIT_LOGS at most VIEW. No permission today sits above VIEW in AUDIT_LOGS, so every set passes rule 1;
/// this pins that `AuditLogsRead` alone is allowed where rules 2 and 3 allow it.
#[test]
fn rule_1_audit_logs_view_is_allowed() {
    assert!(module_permissions(Module::AuditLogs, Level::View).is_empty());
    assert_eq!(
        MatrixGuardrails::check(
            &custom("AUDITOR", Role::VerificationReviewer),
            &set(Role::VerificationReviewer, &[P::AuditLogsRead])
        ),
        Ok(())
    );
}

/// Order after rule 4: 1, 2, 3, 5. An ADMIN-based set that breaks rule 3 and rule 5 answers 3.
#[test]
fn rules_are_answered_in_order() {
    let it = custom("IT", Role::Admin);
    // §11.2 "Branch cash basics": CashManage is now no-module, so use FinanceReconcile (FINANCE FULL) to break rule 3.
    let mut both = set(Role::Admin, &[P::FinanceReconcile]);
    both.retain(|p| *p != P::SignedIn);
    assert_eq!(
        MatrixGuardrails::check(&it, &both),
        Err(GuardrailBreach::AdminBaseAboveView)
    );

    let night = custom("NIGHT_TELLER", Role::Teller);
    let mut two_and_five = set(Role::Teller, &[P::UsersManage]);
    two_and_five.retain(|p| *p != P::SignedIn);
    assert_eq!(
        MatrixGuardrails::check(&night, &two_and_five),
        Err(GuardrailBreach::AdminModulesAboveView)
    );
}
