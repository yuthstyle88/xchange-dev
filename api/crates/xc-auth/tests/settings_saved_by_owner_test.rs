//! Business settings are saved by EXECUTIVE only and read by every staff role (design
//! `docs/design/blueprint/user-management-rbac.md` §11.11, PR #5999, with the lane controller's rulings of 2026-11-02:
//! RATE_MANAGER and ADMIN are in scope; `ConsentPolicyManage` is a setting; `OrgStartingPricesManage` is a setting,
//! since a starting price bounds the drafts, `rates.md` §12.6). `RatePolicySave` and the commands that inherit it stay
//! with RATE_MANAGER (ruling A, #564 issuecomment-6587097622): every bound in them, including `pricingModeSave` and
//! `marginGuardSet`, already needs a valid EXECUTIVE row in the command (`may_change_the_frame`), and the day's margins
//! and `pricingApprove` are the rate manager's work product.
//!
//! Pinned here, through `Policy::authorize` on the generated area rows (no database):
//! - S: every settings save refuses every built-in staff role but EXECUTIVE, and the Owner account (ADMIN+EXECUTIVE)
//!   passes it;
//! - R: every settings read passes every built-in staff role, including a TELLER whose row is at one branch;
//! - O: the daily operations stay with their holders (ADMIN's booking operations, device pairing and a terminal's
//!   printer and drawer, FINANCE's reconciliation, RATE_MANAGER's drafts and rate policy);
//! - S3: the payment methods the shop takes are a setting too: saved by EXECUTIVE only, listed by anybody signed in;
//! - V: the seed, version 10: the old holders lose the saves, every staff group gains `SettingsRead`, and EXECUTIVE
//!   alone holds the saves.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. `[required]` names: `Permission::SettingsRead` and `Permission::BranchSettingsSave`, both with no
//! module; the read rows `transferPolicyRead`, `exchangeSettingsRead`, `marginGuardRead` and `consentTexts`.
//!
//! `EntryPath::DeviceSetup` and the `Devices` actions `setPrinter` and `setDrawer` stay with ADMIN through
//! `BranchesManage` (the ticket author's proposal on #564 issuecomment-6581756064, while `SystemSettings` becomes
//! EXECUTIVE-only).

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::permission_module::PermissionModule;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::areas::{AREA_ROWS, RowScope};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

/// Every settings save (§11.11 "Verdicts", plus the commands that inherit them).
const SAVES: &[&str] = &[
    // Finance/Full today: FINANCE held these.
    "cashLimitsSave",
    "transferPolicySave",
    "dayTarget",
    // RatesStock/Full today: RATE_MANAGER held these.
    "rateAlertSettingsSave",
    "orgStartingPriceSave",
    // SystemSettings/Full today: ADMIN held these.
    "exchangeSettingsSave",
    "appSettingsSave",
    // EXECUTIVE-only by seed already; no module after §11.11.
    "branchAlertSettingsSave",
    // `BranchSettingsSave`, taken from `BranchesManage`, which ADMIN holds.
    "bookingPolicySet",
    "slotRuleSet",
    "slotCapacitySet",
    "branchExchangeLimitSave",
    // Customers/Full today: RATE_MANAGER and BRANCH_MANAGER held it (the controller's ruling: a setting).
    "consentPolicy",
    // #6273: the kiosk screen values (`SystemSettings`), added by the #6273 test writer claude/37e371ca.
    "kioskScreenSave",
];

/// Every settings read (`SettingsRead`).
const READS: &[&str] = &[
    "cashLimitsRead",
    "rateAlertSettings",
    "orgStartingPrices",
    "branchAlertSettings",
    "transferPolicyRead",
    "exchangeSettingsRead",
    "marginGuardRead",
    "consentTexts",
    // #6273: the kiosk screen values' view (`SettingsRead`).
    "kioskScreen",
];

/// The built-in staff groups: every built-in group but CUSTOMER.
const STAFF: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap()
}

/// `roles`, each organisation-wide, signed in at branch B1.
fn org_wide(roles: &[Role]) -> Actor {
    let rows = roles
        .iter()
        .map(|&r| ScopedRole::new(r, None, now() - Duration::days(1), None))
        .collect();
    Actor::new("user-1", ORG, rows, None).with_branch(Some(BranchId::new("B1")))
}

/// `role` with one row at branch B1, signed in there.
fn at_b1(role: Role) -> Actor {
    let row = ScopedRole::new(
        role,
        Some(BranchId::new("B1")),
        now() - Duration::days(1),
        None,
    );
    Actor::new("user-1", ORG, vec![row], None).with_branch(Some(BranchId::new("B1")))
}

fn scope_of(action: &str) -> RowScope {
    AREA_ROWS
        .iter()
        .flat_map(|area| area.iter())
        .find(|row| row.path == EntryPath::Rpc && row.action == action)
        .unwrap_or_else(|| panic!("no Rpc area row for {action}"))
        .scope
}

/// Asks `Policy::authorize` for `action` on `Rpc`, naming B1 on a `Request` row and nothing on an `Organisation` row.
fn ask(actor: &Actor, action: &str) -> Result<(), PolicyError> {
    let b1 = BranchId::new("B1");
    let named = match scope_of(action) {
        RowScope::Request => Some(&b1),
        RowScope::Organisation => None,
    };
    Policy::authorize(
        Some(actor),
        EntryPath::Rpc,
        &Action::new(action),
        named,
        now(),
    )
    .map(|_| ())
}

fn is_permission_refusal(r: &Result<(), PolicyError>) -> bool {
    matches!(
        r,
        Err(PolicyError::RoleNotAllowed) | Err(PolicyError::RoleNotAllowedHere)
    )
}

/// S1: no built-in staff group but EXECUTIVE passes any settings save, even organisation-wide.
#[test]
fn s1_every_settings_save_refuses_every_staff_group_but_executive() {
    for &action in SAVES {
        for role in STAFF.into_iter().filter(|&r| r != Role::Executive) {
            let r = ask(&org_wide(&[role]), action);
            assert!(
                is_permission_refusal(&r),
                "{action}: {role:?} must be refused for the permission, got {r:?}"
            );
        }
    }
}

/// S2: EXECUTIVE alone, and the Owner account (ADMIN+EXECUTIVE), pass every settings save.
#[test]
fn s2_the_owner_passes_every_settings_save() {
    for &action in SAVES {
        assert_eq!(
            ask(&org_wide(&[Role::Executive]), action),
            Ok(()),
            "{action}: EXECUTIVE"
        );
        assert_eq!(
            ask(&org_wide(&[Role::Admin, Role::Executive]), action),
            Ok(()),
            "{action}: the Owner account"
        );
    }
}

/// R1: every staff group reads every setting, organisation-wide.
#[test]
fn r1_every_staff_group_reads_every_setting() {
    for &action in READS {
        for role in STAFF {
            assert_eq!(
                ask(&org_wide(&[role]), action),
                Ok(()),
                "{action}: {role:?}"
            );
        }
    }
}

/// R2: a TELLER and a FINANCE user whose rows are at one branch read every setting there: a read acts on nothing, so
/// no read row is `RowScope::Organisation`.
#[test]
fn r2_branch_staff_read_every_setting_at_their_branch() {
    for &action in READS {
        assert_eq!(
            scope_of(action),
            RowScope::Request,
            "{action}: a read row checks the request's target"
        );
        for role in [Role::Teller, Role::Finance] {
            assert_eq!(
                ask(&at_b1(role), action),
                Ok(()),
                "{action}: {role:?} at B1"
            );
        }
    }
}

/// O1: the daily operations stay with their holders.
#[test]
fn o1_the_daily_operations_stay_with_their_holders() {
    let kept: &[(Role, &str)] = &[
        (Role::Admin, "closureAdd"),
        (Role::Admin, "closureRemove"),
        (Role::Admin, "slotOpen"),
        (Role::Admin, "slotClose"),
        (Role::Admin, "admin_booking_slots"),
        (Role::Finance, "transferReconcile"),
        (Role::Finance, "financeReconciliation"),
        (Role::RateManager, "rateDraft"),
        // The lane controller's ruling A (#564 issuecomment-6587097622): `RatePolicySave` and its inheritors stay with
        // RATE_MANAGER; the bounds in them are EXECUTIVE-only through `may_change_the_frame`.
        (Role::RateManager, "ratePolicySave"),
    ];
    for &(role, action) in kept {
        assert_eq!(
            ask(&org_wide(&[role]), action),
            Ok(()),
            "{action}: {role:?} keeps it"
        );
    }
}

/// O2: device setup stays with ADMIN, through `BranchesManage`, while `SystemSettings` becomes EXECUTIVE-only.
#[test]
fn o2_admin_keeps_device_setup() {
    let admin = org_wide(&[Role::Admin]);
    let b1 = BranchId::new("B1");
    assert_eq!(
        Policy::authorize(
            Some(&admin),
            EntryPath::DeviceSetup,
            &Action::new("pair"),
            Some(&b1),
            now()
        )
        .map(|_| ()),
        Ok(())
    );
}

/// O3: a terminal's printer and drawer stay with ADMIN, through `BranchesManage`, like device pairing.
#[test]
fn o3_admin_keeps_a_terminals_printer_and_drawer() {
    let admin = org_wide(&[Role::Admin]);
    let b1 = BranchId::new("B1");
    for name in ["setPrinter", "setDrawer"] {
        assert_eq!(
            Policy::authorize(
                Some(&admin),
                EntryPath::Devices,
                &Action::new(name),
                Some(&b1),
                now()
            )
            .map(|_| ()),
            Ok(()),
            "{name}"
        );
    }
}

/// S3: the payment methods are saved by EXECUTIVE only (ADMIN held it through `SystemSettings`), and listed by every
/// staff group.
#[test]
fn s3_the_payment_methods_are_saved_by_the_owner_and_listed_by_everyone() {
    let b1 = BranchId::new("B1");
    let ask_pm = |actor: &Actor, action: &str| {
        Policy::authorize(
            Some(actor),
            EntryPath::PaymentMethods,
            &Action::new(action),
            Some(&b1),
            now(),
        )
        .map(|_| ())
    };
    for role in STAFF {
        let save = ask_pm(&org_wide(&[role]), "save");
        if role == Role::Executive {
            assert_eq!(save, Ok(()), "EXECUTIVE saves");
        } else {
            assert!(
                save.is_err(),
                "{role:?} must be refused the save, got {save:?}"
            );
        }
        assert_eq!(ask_pm(&org_wide(&[role]), "list"), Ok(()), "{role:?} lists");
    }
}

/// The settings-save permissions §11.11 makes EXECUTIVE-only, and who held each at seed version 9.
const SAVE_PERMISSIONS: &[(Permission, &[Role])] = &[
    (Permission::CashLimitsManage, &[Role::Finance]),
    (Permission::CashTransferPolicySet, &[Role::Finance]),
    (Permission::CashTargetSet, &[Role::Finance]),
    (Permission::RateAlertSettingsSave, &[Role::RateManager]),
    (Permission::OrgStartingPricesManage, &[Role::RateManager]),
    (Permission::SystemSettings, &[Role::Admin]),
    (
        Permission::ConsentPolicyManage,
        &[Role::RateManager, Role::BranchManager],
    ),
    (Permission::AlertSettingsSave, &[]),
    (Permission::BranchSettingsSave, &[]),
];

/// V1: each settings save has no module, so no module level implies it; `SettingsRead` has none either.
#[test]
fn v1_the_settings_permissions_have_no_module() {
    for &(p, _) in SAVE_PERMISSIONS {
        assert_eq!(PermissionModule::module(p), None, "{p:?}");
    }
    assert_eq!(PermissionModule::module(Permission::SettingsRead), None);
}

/// V2: the seed: EXECUTIVE alone holds every settings save, and every staff group holds `SettingsRead`.
#[test]
fn v2_the_seed_gives_the_saves_to_executive_and_the_read_to_every_staff_group() {
    for &(p, _) in SAVE_PERMISSIONS {
        for role in STAFF {
            assert_eq!(
                RoleGrants::seed(role).contains(&p),
                role == Role::Executive,
                "{p:?} in the seed of {role:?}"
            );
        }
    }
    for role in STAFF {
        assert!(
            RoleGrants::seed(role).contains(&Permission::SettingsRead),
            "SettingsRead in the seed of {role:?}"
        );
    }
    assert!(!RoleGrants::seed(Role::Customer).contains(&Permission::SettingsRead));
}

/// V3: seed version 10 removes the saves from the old holders and adds the two new permissions, so groups seeded at
/// version 9 or earlier are brought along at boot (§11.5 item 2); custom groups are not touched.
#[test]
fn v3_seed_version_10_records_the_removals_and_additions() {
    // Restated for backdated keying (backdated-keying.md §3): SEED_VERSION is 11 (ExchangeBackdate).
    assert_eq!(RoleGrants::SEED_VERSION, 11);
    for &(p, holders) in SAVE_PERMISSIONS {
        if holders.is_empty() {
            continue;
        }
        assert!(
            RoleGrants::SEED_REMOVALS.contains(&(10, p)),
            "SEED_REMOVALS lists (10, {p:?})"
        );
    }
    for p in [Permission::SettingsRead, Permission::BranchSettingsSave] {
        assert!(
            RoleGrants::SEED_ADDITIONS.contains(&(10, p)),
            "SEED_ADDITIONS lists (10, {p:?})"
        );
    }
}
