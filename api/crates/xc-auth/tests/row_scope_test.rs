//! #5526 (design `docs/design/org-wide-actor-branch.md` §10.2, §10.4 rows W-1..W-5): an area row says what its action
//! acts on. `RowScope::Request` keeps the target the request names; `RowScope::Organisation` checks
//! `Target::organisation`. Written with the skeleton by the ticket author (claude/2ca5874b). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Pure: no database. The time is passed in, never read from the clock.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::areas::{AREA_ROWS, AreaRow, RowScope};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

/// Test-only rows; neither action has a row in the table.
// Restated for #6004 (user-management-rbac.md §11.11): SystemSettings is EXECUTIVE only; use BranchesManage to test ADMIN org-level and request-scoped rows.
const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "zzRequestSettings",
        need: Need::Permission(Permission::BranchesManage),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "zzOrganisationSettings",
        need: Need::Permission(Permission::BranchesManage),
        scope: RowScope::Organisation,
    },
];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap()
}

fn admin(row_branch: Option<&str>, session: Option<&str>) -> Actor {
    let row = ScopedRole::new(
        Role::Admin,
        row_branch.map(BranchId::new),
        now() - Duration::days(1),
        None,
    );
    Actor::new("user-1", ORG, vec![row], None).with_branch(session.map(BranchId::new))
}

fn ask(a: &Actor, action: &str, named: Option<&str>) -> Result<BranchId, PolicyError> {
    let named = named.map(BranchId::new);
    Policy::authorize_with(
        &[ROWS],
        Some(a),
        EntryPath::Rpc,
        &Action::new(action),
        named.as_ref(),
        now(),
    )
    .map(|g| g.branch)
}

#[test]
fn w1_an_organisation_row_refuses_a_branch_scoped_admin_in_its_session_named_or_not() {
    let a = admin(Some("B1"), Some("B1"));
    assert_eq!(
        ask(&a, "zzOrganisationSettings", None),
        Err(PolicyError::OutOfScope)
    );
    assert_eq!(
        ask(&a, "zzOrganisationSettings", Some("B1")),
        Err(PolicyError::OutOfScope)
    );
}

#[test]
fn w2_an_organisation_row_grants_an_organisation_wide_admin_at_the_session_branch_whatever_is_named()
 {
    let a = admin(None, Some("B1"));
    assert_eq!(
        ask(&a, "zzOrganisationSettings", None),
        Ok(BranchId::new("B1"))
    );
    assert_eq!(
        ask(&a, "zzOrganisationSettings", Some("B2")),
        Ok(BranchId::new("B1"))
    );
    assert_eq!(
        ask(&admin(None, None), "zzOrganisationSettings", None),
        Err(PolicyError::OutOfScope)
    );
}

#[test]
fn w3_a_request_row_answers_as_before_the_scope_field() {
    let branch_admin = admin(Some("B1"), Some("B1"));
    assert_eq!(
        ask(&branch_admin, "zzRequestSettings", None),
        Ok(BranchId::new("B1"))
    );
    assert_eq!(
        ask(&branch_admin, "zzRequestSettings", Some("B2")),
        Err(PolicyError::OutOfScope)
    );
    assert_eq!(
        ask(&admin(None, Some("B1")), "zzRequestSettings", Some("B2")),
        Ok(BranchId::new("B2"))
    );
}

#[test]
fn w4_a_route_table_row_and_a_missing_row_are_request() {
    assert_eq!(
        Policy::scope_with(AREA_ROWS, EntryPath::Rpc, &Action::new("adminSnapshot")),
        RowScope::Request
    );
    assert_eq!(
        Policy::scope_with(&[ROWS], EntryPath::Rpc, &Action::new("zzNoSuchAction")),
        RowScope::Request
    );
    assert_eq!(
        Policy::scope_with(
            &[ROWS],
            EntryPath::Rpc,
            &Action::new("zzOrganisationSettings")
        ),
        RowScope::Organisation
    );
}

#[test]
fn w5_no_organisation_row_is_public_or_needs_the_customers_own_record() {
    for row in AREA_ROWS.iter().flat_map(|area| area.iter()) {
        if row.scope != RowScope::Organisation {
            continue;
        }
        let customer_or_public = match row.need {
            Need::Public => true,
            Need::Permission(p) => p == Permission::CustomerSelf,
            Need::AnyOf(ps) => ps.contains(&Permission::CustomerSelf),
        };
        assert!(!customer_or_public, "{:?} {}", row.path, row.action);
    }
}
