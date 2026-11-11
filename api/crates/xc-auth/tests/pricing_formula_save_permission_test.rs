//! #5733 (owner ruling A on #5683, 2026-10-30; `docs/design/blueprint/pricing-formula.md` §5 F-2 *"EXECUTIVE
//! เท่านั้น"*): who may save the organisation's price formula is the permission `PricingFormulaSave`, seeded on
//! EXECUTIVE only and editable on the `/users` matrix, not a role name in code.
//! Written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Pure: no database, the seeded snapshot only (nothing here calls `RoleGrants::install`). The time is passed in.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::permission_module::PermissionModule;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::areas::{RowScope, rates};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const P: Permission = Permission::PricingFormulaSave;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap()
}

/// An organisation-wide row of the built-in group `role`, signed in at MAIN.
fn org_wide(role: Role) -> Actor {
    let row = ScopedRole::new(role, None::<BranchId>, now() - Duration::days(1), None);
    Actor::new("user-1", ORG, vec![row], None).with_branch(Some(BranchId::new("MAIN")))
}

fn save_as(role: Role) -> Result<BranchId, PolicyError> {
    Policy::authorize(
        Some(&org_wide(role)),
        EntryPath::Rpc,
        &Action::new("pricingFormulaSave"),
        None,
        now(),
    )
    .map(|g| g.branch)
}

/// F-1: EXECUTIVE is the only built-in group whose seed holds it.
#[test]
fn f1_pricing_formula_save_is_seeded_on_executive_only() {
    let holders: Vec<Role> = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|&r| RoleGrants::seed(r).contains(&P))
        .collect();
    assert_eq!(holders, vec![Role::Executive]);
}

/// F-2: it enters the seed at version 9, once, and is never removed (`SEED_VERSION` 8 -> 9).
#[test]
fn f2_it_enters_the_seed_at_version_9_once_and_is_never_removed() {
    let at: Vec<i32> = RoleGrants::SEED_ADDITIONS
        .iter()
        .filter(|&&(_, q)| q == P)
        .map(|&(v, _)| v)
        .collect();
    assert_eq!(
        at,
        vec![9],
        "SEED_ADDITIONS has (9, PricingFormulaSave) once"
    );
    // Restated for #6004 (user-management-rbac.md §11.11): SEED_VERSION is 10; (9, PricingFormulaSave) is unchanged.
    // Restated for backdated keying (backdated-keying.md §3): SEED_VERSION is 11 (ExchangeBackdate).
    // Restated for XC Counter kiosk exit (#6569): SEED_VERSION is 12 (CounterUnlock).
    assert_eq!(RoleGrants::SEED_VERSION, 12);
    assert!(
        RoleGrants::SEED_REMOVALS.iter().all(|&(_, q)| q != P),
        "it is never removed"
    );
}

/// F-3: no module, so RatesStock at Full (RATE_MANAGER's seed) never implies it.
#[test]
fn f3_it_has_no_module() {
    assert_eq!(PermissionModule::module(P), None);
    assert_eq!(
        Permission::ALL.iter().filter(|&&q| q == P).count(),
        1,
        "in Permission::ALL once"
    );
}

/// F-4: the `pricingFormulaSave` row asks for it and stays an organisation-level row (T1, #5679).
#[test]
fn f4_the_pricing_formula_save_row_needs_it_and_stays_organisation() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("pricingFormulaSave")),
        Some(Need::Permission(P))
    );
    let row = rates::ROWS
        .iter()
        .find(|r| r.path == EntryPath::Rpc && r.action == "pricingFormulaSave")
        .expect("the pricingFormulaSave row");
    assert_eq!(row.scope, RowScope::Organisation);
}

/// F-5: an organisation-wide RATE_MANAGER (it holds `RatePolicySave`) is refused at the seam, and an
/// organisation-wide EXECUTIVE passes.
#[test]
fn f5_an_org_wide_rate_manager_is_refused_and_an_org_wide_executive_passes() {
    assert!(
        RoleGrants::seed(Role::RateManager).contains(&Permission::RatePolicySave),
        "precondition: RATE_MANAGER holds RatePolicySave"
    );
    assert_eq!(
        save_as(Role::RateManager),
        Err(PolicyError::RoleNotAllowedHere)
    );
    assert!(
        save_as(Role::Executive).is_ok(),
        "{:?}",
        save_as(Role::Executive)
    );
}
