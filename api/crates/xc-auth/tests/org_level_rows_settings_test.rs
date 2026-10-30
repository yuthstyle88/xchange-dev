//! #5526 T1 (design `docs/design/org-wide-actor-branch.md` §10.4 rows A-1..A-5): the rows of the organisation settings and policies act on an
//! object of the whole organisation, so `Policy::authorize` checks them against `Target::organisation`, which only an
//! organisation-wide row passes. Area files: `areas/bookings.rs`, `areas/cash_transfer_policy.rs`, `areas/customer_app.rs`, `areas/org_starting_prices.rs`, `areas/rate_alerts.rs`. Written before the implementation by the ticket author
//! (claude/2ca5874b). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Pure: no database. The time is passed in, never read from the clock. Each case takes the first built-in group that
//! holds the row's permission (`RoleGrants::holds`), so the tests follow the seeded grants rather than naming roles.
//!
//! Signatures these tests assume (§10.2 [required]): `AreaRow.scope: RowScope`, `RowScope::{Request, Organisation}`
//! in `xc_auth::domain::policy::areas`; `Policy::authorize` unchanged.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::policy::areas::{
    AreaRow, RowScope, bookings, cash_transfer_policy, customer_app, org_starting_prices,
    rate_alerts,
};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

/// The area files this ticket owns.
const AREAS: &[&[AreaRow]] = &[
    bookings::ROWS,
    cash_transfer_policy::ROWS,
    customer_app::ROWS,
    org_starting_prices::ROWS,
    rate_alerts::ROWS,
];

/// Every row of these files whose object is the whole organisation (§10.3), in file order.
const ORGANISATION_ROWS: &[(EntryPath, &str)] = &[
    (EntryPath::Rpc, "bookingPolicySet"),
    (EntryPath::Rpc, "transferPolicySave"),
    (EntryPath::Rpc, "appSettingsSave"),
    (EntryPath::Rpc, "orgStartingPriceSave"),
    (EntryPath::Rpc, "orgStartingPrices"),
    (EntryPath::Rpc, "rateAlertSettingsSave"),
    (EntryPath::Rpc, "rateAlertSettings"),
];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap()
}

fn permission_of(path: EntryPath, action: &str) -> Permission {
    match Policy::need(path, &Action::new(action)) {
        Some(Need::Permission(p)) => p,
        Some(Need::AnyOf(ps)) => ps[0],
        other => panic!("{path:?} {action}: expected a permission row, got {other:?}"),
    }
}

/// The first built-in group that holds `permission`.
fn holder(permission: Permission) -> Role {
    RoleGrants::BUILT_IN
        .into_iter()
        .find(|r| RoleGrants::holds(r.as_str(), permission))
        .unwrap_or_else(|| panic!("no built-in group holds {permission:?}"))
}

fn actor(role: Role, row_branch: Option<&str>, session: Option<&str>) -> Actor {
    let row = ScopedRole::new(
        role,
        row_branch.map(BranchId::new),
        now() - Duration::days(1),
        None,
    );
    Actor::new("user-1", ORG, vec![row], None).with_branch(session.map(BranchId::new))
}

fn ask(
    path: EntryPath,
    action: &str,
    row_branch: Option<&str>,
    session: Option<&str>,
    named: Option<&str>,
) -> Result<BranchId, PolicyError> {
    let a = actor(holder(permission_of(path, action)), row_branch, session);
    let named = named.map(BranchId::new);
    Policy::authorize(Some(&a), path, &Action::new(action), named.as_ref(), now()).map(|g| g.branch)
}

#[test]
fn a1_the_rows_marked_organisation_are_exactly_these() {
    let marked: Vec<(EntryPath, &str)> = AREAS
        .iter()
        .flat_map(|area| area.iter())
        .filter(|row| row.scope == RowScope::Organisation)
        .map(|row| (row.path, row.action))
        .collect();
    assert_eq!(marked, ORGANISATION_ROWS);
}

#[test]
fn a2_a_branch_scoped_holder_in_its_session_is_refused_each_row_naming_no_branch() {
    for &(path, action) in ORGANISATION_ROWS {
        assert_eq!(
            ask(path, action, Some("B1"), Some("B1"), None),
            Err(PolicyError::OutOfScope),
            "{path:?} {action}"
        );
    }
}

#[test]
fn a3_a_branch_scoped_holder_naming_its_own_branch_is_refused_each_row() {
    for &(path, action) in ORGANISATION_ROWS {
        assert_eq!(
            ask(path, action, Some("B1"), Some("B1"), Some("B1")),
            Err(PolicyError::OutOfScope),
            "{path:?} {action}"
        );
    }
}

#[test]
fn a4_an_organisation_wide_holder_in_a_session_is_granted_each_row_at_the_session_branch() {
    for &(path, action) in ORGANISATION_ROWS {
        assert_eq!(
            ask(path, action, None, Some("B1"), Some("B2")),
            Ok(BranchId::new("B1")),
            "{path:?} {action}"
        );
    }
}

#[test]
fn a5_the_other_rows_of_these_files_still_grant_a_branch_scoped_holder_at_its_session_branch() {
    for row in AREAS.iter().flat_map(|area| area.iter()) {
        if ORGANISATION_ROWS.contains(&(row.path, row.action)) {
            continue;
        }
        let permission = match row.need {
            Need::Permission(p) => p,
            Need::AnyOf(ps) => ps[0],
            _ => continue,
        };
        if matches!(permission, Permission::CustomerSelf | Permission::SignedIn) {
            continue;
        }
        assert_eq!(
            ask(row.path, row.action, Some("B1"), Some("B1"), None),
            Ok(BranchId::new("B1")),
            "{:?} {}",
            row.path,
            row.action
        );
    }
}
