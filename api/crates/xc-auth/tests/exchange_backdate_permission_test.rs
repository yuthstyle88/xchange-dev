//! Backdated keying (`docs/design/backdated-keying.md` §3; owner rule R1 of 2026-11-04 *"Owner and Manager roles
//! only"*): `transactionBackdate` needs `ExchangeBackdate`, seeded on EXECUTIVE (the Owner holds it) and
//! BRANCH_MANAGER only, at seed v11.
//! Written before the implementation by the sub-controller claude/6bb512ce. Do not edit, delete or weaken;
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
use xc_auth::domain::policy::areas::{RowScope, backdated_keying};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const P: Permission = Permission::ExchangeBackdate;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 13, 0, 0).unwrap()
}

/// A row of the built-in group `role` at `branch` (`None` = organisation-wide), signed in at MAIN.
fn caller(role: Role, branch: Option<&str>) -> Actor {
    let row = ScopedRole::new(
        role,
        branch.map(BranchId::new),
        now() - Duration::days(1),
        None,
    );
    Actor::new("user-1", ORG, vec![row], None).with_branch(Some(BranchId::new("MAIN")))
}

fn backdate_as(who: &Actor) -> Result<BranchId, PolicyError> {
    Policy::authorize(
        Some(who),
        EntryPath::Rpc,
        &Action::new("transactionBackdate"),
        None,
        now(),
    )
    .map(|g| g.branch)
}

/// BP-1: EXECUTIVE and BRANCH_MANAGER are the only built-in groups whose seed holds it.
#[test]
fn bp1_exchange_backdate_is_seeded_on_executive_and_branch_manager_only() {
    let holders: Vec<Role> = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|&r| RoleGrants::seed(r).contains(&P))
        .collect();
    assert_eq!(holders, vec![Role::Executive, Role::BranchManager]);
}

/// BP-2: it enters the seed at version 11, once, and is never removed (`SEED_VERSION` 10 -> 11).
#[test]
fn bp2_it_enters_the_seed_at_version_11_once_and_is_never_removed() {
    let at: Vec<i32> = RoleGrants::SEED_ADDITIONS
        .iter()
        .filter(|&&(_, q)| q == P)
        .map(|&(v, _)| v)
        .collect();
    assert_eq!(
        at,
        vec![11],
        "SEED_ADDITIONS has (11, ExchangeBackdate) once"
    );
    assert_eq!(RoleGrants::SEED_VERSION, 11);
    assert!(
        RoleGrants::SEED_REMOVALS.iter().all(|&(_, q)| q != P),
        "it is never removed"
    );
}

/// BP-3: no module, so no module level (ExchangeDesk at Full included) implies it; in `ALL` once.
#[test]
fn bp3_it_has_no_module_and_is_in_all_once() {
    assert_eq!(PermissionModule::module(P), None);
    assert_eq!(
        Permission::ALL.iter().filter(|&&q| q == P).count(),
        1,
        "in Permission::ALL once"
    );
}

/// BP-4: the `transactionBackdate` row asks for it, scoped to the request's branch.
#[test]
fn bp4_the_transaction_backdate_row_needs_it() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("transactionBackdate")),
        Some(Need::Permission(P))
    );
    let row = backdated_keying::ROWS
        .iter()
        .find(|r| r.path == EntryPath::Rpc && r.action == "transactionBackdate")
        .expect("the transactionBackdate row");
    assert_eq!(row.scope, RowScope::Request);
}

/// BP-5 (B2, B3): a MAIN teller is refused at the seam; a MAIN branch manager and an organisation-wide
/// EXECUTIVE pass.
#[test]
fn bp5_a_teller_is_refused_and_a_branch_manager_and_an_executive_pass() {
    assert_eq!(
        backdate_as(&caller(Role::Teller, Some("MAIN"))),
        Err(PolicyError::RoleNotAllowedHere)
    );
    assert!(
        backdate_as(&caller(Role::BranchManager, Some("MAIN"))).is_ok(),
        "{:?}",
        backdate_as(&caller(Role::BranchManager, Some("MAIN")))
    );
    assert!(
        backdate_as(&caller(Role::Executive, None)).is_ok(),
        "{:?}",
        backdate_as(&caller(Role::Executive, None))
    );
}

/// BP-6: the other staff groups (ADMIN alone, RATE_MANAGER, FINANCE, MARKETING, VERIFICATION_REVIEWER) are refused.
#[test]
fn bp6_the_other_staff_groups_are_refused() {
    for role in [
        Role::Admin,
        Role::RateManager,
        Role::Finance,
        Role::Marketing,
        Role::VerificationReviewer,
    ] {
        assert_eq!(
            backdate_as(&caller(role, Some("MAIN"))),
            Err(PolicyError::RoleNotAllowedHere),
            "{role:?}"
        );
    }
}
