//! Up-front tests for RBAC §11 ticket C6a (design `docs/design/blueprint/user-management-rbac.md` §11.2, §11.4, §11.7):
//! the seed, the snapshot, the desk basics and the wiring of `Authorizer::check` to a group's set.
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! No database. `RoleGrants::install` is process-global, so every test here holds `LOCK` and puts the seeded
//! snapshot back before it ends.
//!
//! The seed lists below are §11.2's table applied through `PermissionModule::module`, with the desk basics of
//! §11.2 **and** the five desk device and document permissions as no-module (claude/65357afb's proposal A,
//! #564 issuecomment-6261275325). If the BE controller rules otherwise, the sub-controller restates this file.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use chrono::{Duration, Utc};
use xc_auth::domain::access::permission_module::{
    Level, Module, PermissionMatrix, PermissionModule,
};
use xc_auth::domain::access::{
    Actor, Authorizer, Group, Permission, Refusal, Role, RoleGrants, RoleGrantsSnapshot,
    ScopedRole, Target,
};
use xc_kernel::ids::BranchId;

use Permission as P;

static LOCK: Mutex<()> = Mutex::new(());

fn locked() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Puts the seeded snapshot back when a test passes. A failing test leaves it (installing while unwinding would
/// abort the binary); every test that reads the process snapshot installs what it needs first.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            RoleGrants::install(RoleGrantsSnapshot::seeded());
        }
    }
}

/// §11.2's seed, per built-in group, in `Permission::ALL` order.
const SEED: [(Role, &[Permission]); 9] = [
    (
        Role::Executive,
        &[
            P::OverviewRead,
            P::RatesDraft,
            P::RatesHistoryRead,
            P::RatePolicySave,
            P::BranchesManage,
            P::UsersManage,
            P::ApprovalsDecide,
            P::BookingsDesk,
            P::ExchangeTransact,
            P::TransactionRead,
            P::ReceiptReprint,
            P::ReversalRequest,
            P::CashManage,
            P::QueueOperate,
            P::SystemSettings,
            P::CompatRead,
            P::RequestAbandon,
            P::DocumentsList,
            P::FileRead,
            P::ProofUpload,
            P::DeviceQueue,
            P::DeviceProcess,
            P::DevicePrinterOnline,
            P::DrawerPassport,
            P::FinanceReconcile,
            P::VerificationReview,
            P::IdentityDocumentRead,
            P::MarketingCampaigns,
            P::MarketingMembers,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::CustomerHoldPlace,
            P::CashTargetSet,
            P::SignedIn,
            P::ReferenceRatesRead,
            P::ConsentPolicyManage,
            P::ReferralAttach,
            P::ReferralSupersede,
            P::ReferralRead,
            P::CashLimitsManage,
            P::ReportsExport,
            P::AlertSettingsSave,
            P::CashTransferPolicySet,
            P::RateAlertSettingsSave,
            P::OrgStartingPricesManage,
            P::MarketRatesRefresh,
            P::LoyaltyRulesManage,
            P::AuditLogsRead,
        ],
    ),
    (
        Role::Admin,
        // #4155 W/A1: OverviewRead is EXECUTIVE only; ADMIN holds no DASHBOARD cell here.
        &[
            P::BranchesManage,
            P::UsersManage,
            P::SystemSettings,
            P::CompatRead,
            P::DocumentsList,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::SignedIn,
            P::AuditLogsRead,
        ],
    ),
    (
        Role::RateManager,
        // #4155 W/A1: OverviewRead and AlertSettingsSave are EXECUTIVE only.
        &[
            P::RatesDraft,
            P::RatesHistoryRead,
            P::RatePolicySave,
            P::BookingsDesk,
            P::ExchangeTransact,
            P::TransactionRead,
            P::ReceiptReprint,
            P::ReversalRequest,
            P::QueueOperate,
            P::CompatRead,
            P::DocumentsList,
            P::FileRead,
            P::IdentityDocumentRead,
            P::MarketingCampaigns,
            P::MarketingMembers,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::CustomerHoldPlace,
            P::SignedIn,
            P::ReferenceRatesRead,
            P::ConsentPolicyManage,
            P::ReferralAttach,
            P::ReferralSupersede,
            P::ReferralRead,
            P::ReportsExport,
            P::RateAlertSettingsSave,
            P::OrgStartingPricesManage,
            P::MarketRatesRefresh,
            P::LoyaltyRulesManage,
        ],
    ),
    (
        Role::BranchManager,
        // §11.2 "Branch cash basics": BRANCH_MANAGER holds ApprovalsDecide and CashManage as no-module basics.
        // #4155 W/A1: OverviewRead and AlertSettingsSave are EXECUTIVE only.
        &[
            P::ApprovalsDecide,
            P::ShiftOperate,
            P::ShiftCloseCertify,
            P::BookingsDesk,
            P::ExchangeTransact,
            P::TransactionRead,
            P::ReceiptReprint,
            P::ReversalRequest,
            P::CashManage,
            P::QueueOperate,
            P::CompatRead,
            P::DocumentsList,
            P::FileRead,
            P::DeviceQueue,
            P::DeviceProcess,
            P::DevicePrinterOnline,
            P::DrawerPassport,
            P::VerificationReview,
            P::IdentityDocumentRead,
            P::MarketingCampaigns,
            P::MarketingMembers,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::CustomerHoldPlace,
            P::BranchDayOperate,
            P::SignedIn,
            P::ConsentPolicyManage,
            P::ReferralAttach,
            P::ReferralSupersede,
            P::ReferralRead,
            P::LoyaltyRulesManage,
        ],
    ),
    (
        Role::Teller,
        // #4155 W/A1: OverviewRead is EXECUTIVE only.
        &[
            P::ShiftOperate,
            P::BookingsDesk,
            P::ExchangeTransact,
            P::TransactionRead,
            P::ReceiptReprint,
            P::ReversalRequest,
            P::QueueOperate,
            P::CompatRead,
            P::DocumentsList,
            P::FileRead,
            P::ProofUpload,
            P::DeviceQueue,
            P::DeviceProcess,
            P::DrawerPassport,
            P::IdentityDocumentRead,
            P::AttendanceClock,
            P::CustomerHoldPlace,
            P::SignedIn,
            P::ReferralAttach,
            P::ReferralRead,
        ],
    ),
    (
        Role::Finance,
        // #4155 W/A1: OverviewRead and AlertSettingsSave are EXECUTIVE only.
        &[
            P::RatesHistoryRead,
            P::ApprovalsDecide,
            P::TransactionRead,
            P::ReceiptReprint,
            P::CashManage,
            P::CompatRead,
            P::DocumentsList,
            P::FileRead,
            P::ProofUpload,
            P::FinanceReconcile,
            P::IdentityDocumentRead,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::CashTargetSet,
            P::SignedIn,
            P::ReferenceRatesRead,
            P::ReferralRead,
            P::CashLimitsManage,
            P::ReportsExport,
            P::CashTransferPolicySet,
        ],
    ),
    (
        Role::Marketing,
        &[
            P::MarketingCampaigns,
            P::MarketingMembers,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::SignedIn,
        ],
    ),
    (Role::Customer, &[P::CustomerSelf, P::SignedIn]),
    (
        Role::VerificationReviewer,
        // #4155 W/A1: OverviewRead and AlertSettingsSave are EXECUTIVE only.
        &[
            P::RatesHistoryRead,
            P::TransactionRead,
            P::ReceiptReprint,
            P::CompatRead,
            P::DocumentsList,
            P::FileRead,
            P::VerificationReview,
            P::IdentityDocumentRead,
            P::MarketingPerformance,
            P::AttendanceClock,
            P::SignedIn,
            P::ReferenceRatesRead,
            P::ReferralRead,
            P::ReportsExport,
            P::AuditLogsRead,
        ],
    ),
];

/// The no-module permissions: §11.2's desk basics, "Branch cash basics" (`CashManage`, `ApprovalsDecide`),
/// `SignedIn` and the rest of §11.3 rule 5, and proposal A's five.
const NO_MODULE: [Permission; 16] = [
    P::SignedIn,
    P::AttendanceClock,
    P::CompatRead,
    P::RequestAbandon,
    P::DocumentsList,
    P::CustomerSelf,
    P::ShiftOperate,
    P::BranchDayOperate,
    P::ShiftCloseCertify,
    P::CashManage,
    P::ApprovalsDecide,
    P::DeviceQueue,
    P::DeviceProcess,
    P::DevicePrinterOnline,
    P::DrawerPassport,
    P::ProofUpload,
];

fn in_all_order(set: &[Permission]) -> Vec<Permission> {
    Permission::ALL
        .iter()
        .copied()
        .filter(|p| set.contains(p))
        .collect()
}

#[test]
fn seed_is_the_owners_matrix_per_group() {
    for (role, want) in SEED {
        assert_eq!(
            RoleGrants::seed(role),
            want.to_vec(),
            "seed of {}",
            role.as_str()
        );
    }
}

#[test]
fn seed_is_in_permission_all_order_without_duplicates() {
    for role in RoleGrants::BUILT_IN {
        let got = RoleGrants::seed(role);
        assert_eq!(got, in_all_order(&got), "{} in ALL order", role.as_str());
        let distinct: BTreeSet<_> = got.iter().collect();
        assert_eq!(
            distinct.len(),
            got.len(),
            "{} has no duplicate",
            role.as_str()
        );
    }
}

#[test]
fn desk_basics_and_desk_devices_have_no_module() {
    for p in NO_MODULE {
        assert_eq!(
            PermissionModule::module(p),
            None,
            "{p:?} is not in the matrix"
        );
    }
}

#[test]
fn every_other_permission_keeps_a_module() {
    for p in Permission::ALL {
        if !NO_MODULE.contains(&p) {
            assert!(PermissionModule::module(p).is_some(), "{p:?} has a module");
        }
    }
}

/// §11.2: "The desk basics stay with every desk group." A teller can still open a shift, transact, upload a proof,
/// read a passport and use the device queue after the seed.
#[test]
fn a_teller_keeps_the_desk_after_the_seed() {
    let teller = RoleGrants::seed(Role::Teller);
    for p in [
        P::ShiftOperate,
        P::ExchangeTransact,
        P::BookingsDesk,
        P::QueueOperate,
        P::ProofUpload,
        P::DrawerPassport,
        P::DeviceQueue,
        P::DeviceProcess,
        P::AttendanceClock,
        P::SignedIn,
    ] {
        assert!(teller.contains(&p), "TELLER keeps {p:?}");
    }
    let manager = RoleGrants::seed(Role::BranchManager);
    for p in [
        P::ShiftOperate,
        P::BranchDayOperate,
        P::ShiftCloseCertify,
        P::DevicePrinterOnline,
    ] {
        assert!(manager.contains(&p), "BRANCH_MANAGER keeps {p:?}");
    }
}

/// §11.2's table, read back through `PermissionMatrix::of_group` for every staff group (RISK has no permission yet,
/// so it is never present; MARKETING keeps today's set).
#[test]
fn the_matrix_of_each_built_in_group_is_the_owners_table() {
    let _g = locked();
    let _r = Restore;
    RoleGrants::install(RoleGrantsSnapshot::seeded());
    use Level::{Create as C, Full as F, View as V};
    use Module::*;
    let rows: [(&str, &[(Module, Level)]); 8] = [
        (
            "EXECUTIVE",
            &[
                (Dashboard, F),
                (Customers, F),
                (Documents, F),
                (RatesStock, F),
                (Finance, F),
                (Reports, F),
                (UserManagement, F),
                (SystemSettings, F),
                (AuditLogs, V),
            ],
        ),
        (
            "ADMIN",
            // #4155 W/A1: no DASHBOARD cell outside EXECUTIVE.
            &[
                (Reports, V),
                (UserManagement, F),
                (SystemSettings, F),
                (AuditLogs, V),
            ],
        ),
        (
            "RATE_MANAGER",
            // #4155 W/A1: no DASHBOARD cell outside EXECUTIVE.
            &[
                (Customers, F),
                (Documents, V),
                (RatesStock, F),
                (Reports, F),
            ],
        ),
        (
            "BRANCH_MANAGER",
            // #4155 W/A1: no DASHBOARD cell outside EXECUTIVE.
            &[(Customers, F), (Documents, F), (Reports, V)],
        ),
        // #4155 W/A1: no DASHBOARD cell outside EXECUTIVE.
        ("TELLER", &[(Customers, C), (Documents, V)]),
        (
            "FINANCE",
            // #4155 W/A1: no DASHBOARD cell outside EXECUTIVE.
            &[
                (Customers, V),
                (Documents, V),
                (RatesStock, V),
                (Finance, F),
                (Reports, F),
            ],
        ),
        ("MARKETING", &[(Customers, F), (Reports, V)]),
        (
            "VERIFICATION_REVIEWER",
            // #4155 W/A1: no DASHBOARD cell outside EXECUTIVE.
            &[
                (Customers, V),
                (Documents, F),
                (RatesStock, V),
                (Reports, F),
                (AuditLogs, V),
            ],
        ),
    ];
    for (code, want) in rows {
        let want: BTreeMap<Module, Level> = want.iter().copied().collect();
        assert_eq!(PermissionMatrix::of_group(code), want, "{code}");
    }
    assert!(PermissionMatrix::of_group("NO_SUCH_GROUP").is_empty());
}

#[test]
fn seeded_snapshot_holds_the_nine_built_in_groups_at_version_zero() {
    let s = RoleGrantsSnapshot::seeded();
    assert_eq!(s.version(), 0);
    assert_eq!(s.groups().count(), 9);
    for role in RoleGrants::BUILT_IN {
        let code = role.as_str();
        assert_eq!(
            s.group(code),
            Some(&Group::built_in(role)),
            "{code} is built in, base itself"
        );
        let want: BTreeSet<Permission> = RoleGrants::seed(role).into_iter().collect();
        assert_eq!(s.permissions(code), Some(&want), "{code}'s set is its seed");
    }
    assert_eq!(s.permissions("NO_SUCH_GROUP"), None);
}

#[test]
fn a_later_entry_with_the_same_code_replaces_an_earlier_one() {
    let s = RoleGrantsSnapshot::new(
        3,
        vec![
            (Group::built_in(Role::Teller), vec![P::SignedIn]),
            (
                Group::built_in(Role::Teller),
                vec![P::ExchangeTransact, P::SignedIn],
            ),
        ],
    );
    assert_eq!(s.version(), 3);
    assert_eq!(s.groups().count(), 1);
    assert_eq!(
        s.permissions("TELLER"),
        Some(&BTreeSet::from([P::ExchangeTransact, P::SignedIn]))
    );
}

#[test]
fn until_something_is_installed_the_process_authorises_from_the_seed() {
    let _g = locked();
    let _r = Restore;
    RoleGrants::install(RoleGrantsSnapshot::seeded());
    assert_eq!(*RoleGrants::current(), RoleGrantsSnapshot::seeded());
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            RoleGrants::of(role.as_str()),
            RoleGrants::seed(role),
            "{}",
            role.as_str()
        );
        assert_eq!(RoleGrants::base_of(role.as_str()), Some(role));
    }
}

fn night_teller() -> Group {
    Group {
        code: "NIGHT_TELLER".to_owned(),
        name: "Night teller".to_owned(),
        base: Role::Teller,
        built_in: false,
    }
}

#[test]
fn holds_of_and_base_of_read_the_installed_snapshot() {
    let _g = locked();
    let _r = Restore;
    let mut groups: Vec<(Group, Vec<Permission>)> = RoleGrants::BUILT_IN
        .iter()
        .map(|&r| (Group::built_in(r), RoleGrants::seed(r)))
        .collect();
    groups.push((night_teller(), vec![P::SignedIn, P::ExchangeTransact]));
    RoleGrants::install(RoleGrantsSnapshot::new(7, groups));

    assert_eq!(RoleGrants::current().version(), 7);
    assert!(RoleGrants::holds("NIGHT_TELLER", P::ExchangeTransact));
    assert!(
        !RoleGrants::holds("NIGHT_TELLER", P::QueueOperate),
        "not its set, though its base holds it"
    );
    assert_eq!(
        RoleGrants::of("NIGHT_TELLER"),
        vec![P::ExchangeTransact, P::SignedIn]
    );
    assert_eq!(RoleGrants::base_of("NIGHT_TELLER"), Some(Role::Teller));

    assert!(
        !RoleGrants::holds("NO_SUCH_GROUP", P::SignedIn),
        "an unknown code holds nothing"
    );
    assert!(RoleGrants::of("NO_SUCH_GROUP").is_empty());
    assert_eq!(RoleGrants::base_of("NO_SUCH_GROUP"), None);
}

#[test]
fn a_built_in_code_has_its_base_even_when_the_snapshot_lacks_it() {
    let _g = locked();
    let _r = Restore;
    RoleGrants::install(RoleGrantsSnapshot::new(
        1,
        vec![(night_teller(), vec![P::SignedIn])],
    ));
    assert_eq!(RoleGrants::base_of("FINANCE"), Some(Role::Finance));
    assert!(
        !RoleGrants::holds("FINANCE", P::CashManage),
        "but it holds nothing the snapshot does not give it"
    );
}

const ORG: &str = "org-1";

fn actor(rows: Vec<ScopedRole>) -> Actor {
    Actor::new("u-1", ORG, rows, None)
}

fn at(branch: &str) -> Target {
    Target::new(ORG, Some(BranchId::new(branch)), None)
}

/// §11.7's wiring mutant: an edited set is what `Authorizer::check` sees. `RoleGrants::of` answering the base's seed
/// (or `Authorizer` still reading static role seeds instead of the installed snapshot) turns this red.
#[test]
fn an_edited_set_is_what_authorizer_check_sees() {
    let _g = locked();
    let _r = Restore;
    let now = Utc::now();
    let teller = actor(vec![ScopedRole::new(
        Role::Teller,
        Some(BranchId::new("B1")),
        now - Duration::hours(1),
        None,
    )]);
    RoleGrants::install(RoleGrantsSnapshot::seeded());
    assert!(Authorizer::check(Some(&teller), P::ExchangeTransact, &at("B1"), now).is_ok());

    let groups = RoleGrants::BUILT_IN
        .iter()
        .map(|&r| {
            let mut set = RoleGrants::seed(r);
            if r == Role::Teller {
                set.retain(|p| *p != P::ExchangeTransact);
            }
            (Group::built_in(r), set)
        })
        .collect();
    RoleGrants::install(RoleGrantsSnapshot::new(2, groups));
    assert_eq!(
        Authorizer::check(Some(&teller), P::ExchangeTransact, &at("B1"), now),
        Err(Refusal::RoleNotAllowed),
        "TELLER's edited set no longer holds ExchangeTransact"
    );
}

/// §11.1: an added group behaves as its base for scope, and as its own set for permissions.
#[test]
fn an_added_group_is_scoped_like_a_row_and_allowed_by_its_own_set() {
    let _g = locked();
    let _r = Restore;
    let now = Utc::now();
    let mut groups: Vec<(Group, Vec<Permission>)> = RoleGrants::BUILT_IN
        .iter()
        .map(|&r| (Group::built_in(r), RoleGrants::seed(r)))
        .collect();
    groups.push((
        night_teller(),
        vec![P::SignedIn, P::ExchangeTransact, P::CashManage],
    ));
    RoleGrants::install(RoleGrantsSnapshot::new(4, groups));

    let night = actor(vec![ScopedRole::in_group(
        "NIGHT_TELLER",
        Role::Teller,
        Some(BranchId::new("B1")),
        now - Duration::hours(1),
        None,
    )]);
    let grant = Authorizer::check(Some(&night), P::CashManage, &at("B1"), now)
        .expect("its own set holds CashManage");
    assert_eq!(
        grant.roles_used,
        vec![Role::Teller],
        "the grant names the base"
    );
    assert_eq!(
        Authorizer::check(Some(&night), P::CashManage, &at("B2"), now),
        Err(Refusal::OutOfScope),
        "scoped to its row's branch"
    );
    assert_eq!(
        Authorizer::check(Some(&night), P::QueueOperate, &at("B1"), now),
        Err(Refusal::RoleNotAllowed),
        "its base's seed does not leak into it"
    );
}

/// §11.2 "Branch cash basics" (owner ruling A, #3711): `CashManage` and `ApprovalsDecide` have no module and are
/// seeded on EXECUTIVE, FINANCE and BRANCH_MANAGER, and on none of TELLER, RATE_MANAGER, ADMIN or
/// VERIFICATION_REVIEWER.
#[test]
fn branch_cash_basics_are_no_module_and_seeded_on_the_ruled_bases() {
    for p in [P::CashManage, P::ApprovalsDecide] {
        assert_eq!(
            PermissionModule::module(p),
            None,
            "{p:?} is a no-module branch cash basic"
        );
        for role in [Role::Executive, Role::Finance, Role::BranchManager] {
            assert!(
                RoleGrants::seed(role).contains(&p),
                "{} holds {p:?} in RoleGrants::seed",
                role.as_str()
            );
        }
        for role in [
            Role::Teller,
            Role::RateManager,
            Role::Admin,
            Role::VerificationReviewer,
        ] {
            assert!(
                !RoleGrants::seed(role).contains(&p),
                "{} does not hold {p:?} in RoleGrants::seed",
                role.as_str()
            );
        }
    }
}
