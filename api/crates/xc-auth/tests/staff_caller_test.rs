//! ST-1: who may enter `/staff` (design `docs/design/blueprint/user-management-rbac.md` §11.8).
//!
//! Written before the implementation by the ticket author (claude/7ca80c). Do not edit, delete or weaken these
//! tests; add tests in a file of your own.
//!
//! Signature assumed ([required]): `xc_auth::domain::access::is_staff_caller(actor: &Actor, now: DateTime<Utc>)
//! -> bool`: true iff some role row is valid at `now` (`valid_from <= now`, `valid_to` none or `> now`) and its
//! base `role` is not `Role::Customer`. Rows S1–S8 of §11.8; S9 is the HTTP side, in the ticket's own tests.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole, is_staff_caller};
use xc_kernel::ids::BranchId;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 22, 0, 0, 0).unwrap()
}

fn since() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn org_wide(role: Role) -> ScopedRole {
    ScopedRole::new(role, None, since(), None)
}

fn at(role: Role, branch: &str) -> ScopedRole {
    ScopedRole::new(role, Some(BranchId::new(branch)), since(), None)
}

fn who(roles: Vec<ScopedRole>) -> Actor {
    Actor::new("u-1", "ORG-1", roles, None)
}

#[test]
fn s1_the_owner_executive_and_admin_org_wide_is_a_staff_caller() {
    let owner = who(vec![org_wide(Role::Executive), org_wide(Role::Admin)]);
    assert!(is_staff_caller(&owner, now()));
}

#[test]
fn s2_executive_alone_is_a_staff_caller() {
    assert!(is_staff_caller(
        &who(vec![org_wide(Role::Executive)]),
        now()
    ));
}

#[test]
fn s3_admin_org_wide_alone_is_a_staff_caller() {
    assert!(is_staff_caller(&who(vec![org_wide(Role::Admin)]), now()));
}

#[test]
fn s4_a_teller_at_a_branch_is_a_staff_caller() {
    assert!(is_staff_caller(&who(vec![at(Role::Teller, "B1")]), now()));
}

#[test]
fn s5_a_customer_only_session_is_not_a_staff_caller() {
    let customer = Actor::new("u-2", "ORG-1", vec![at(Role::Customer, "B1")], Some("c-1"));
    assert!(!is_staff_caller(&customer, now()));
}

#[test]
fn s5b_no_roles_at_all_is_not_a_staff_caller() {
    assert!(!is_staff_caller(&who(vec![]), now()));
}

#[test]
fn s7_an_expired_executive_with_nothing_else_is_not_a_staff_caller() {
    let expired = ScopedRole::new(
        Role::Executive,
        None,
        since(),
        Some(now() - Duration::seconds(1)),
    );
    assert!(!is_staff_caller(&who(vec![expired]), now()));
}

#[test]
fn s7b_a_row_that_ends_exactly_now_does_not_count() {
    let ends_now = ScopedRole::new(
        Role::Teller,
        Some(BranchId::new("B1")),
        since(),
        Some(now()),
    );
    assert!(!is_staff_caller(&who(vec![ends_now]), now()));
}

#[test]
fn s7c_a_row_that_starts_in_the_future_does_not_count() {
    let later = ScopedRole::new(
        Role::Teller,
        Some(BranchId::new("B1")),
        now() + Duration::days(1),
        None,
    );
    assert!(!is_staff_caller(&who(vec![later]), now()));
}

#[test]
fn s8_customer_plus_a_teller_row_is_a_staff_caller() {
    let both = Actor::new(
        "u-3",
        "ORG-1",
        vec![at(Role::Customer, "B1"), at(Role::Teller, "B1")],
        Some("c-2"),
    );
    assert!(is_staff_caller(&both, now()));
}

#[test]
fn an_added_group_counts_by_its_base_not_its_code() {
    // §11.1: a new group behaves like its base for every rule the code keeps by role.
    let desk = ScopedRole::in_group(
        "DESK_LEAD",
        Role::Teller,
        Some(BranchId::new("B1")),
        since(),
        None,
    );
    assert!(is_staff_caller(&who(vec![desk]), now()));
}

/// Wiring: `/staff` callers are chosen with `is_staff_caller`, not the fixed `Standing` rule (§11.8, actor.rs:125-127).
#[test]
fn the_session_reader_chooses_staff_callers_with_is_staff_caller() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/presentation/extractors/actor.rs"
    );
    let src = std::fs::read_to_string(path).expect("actor.rs is readable");
    assert!(
        src.contains("is_staff_caller("),
        "AccountSessionReader::callers must pick staff_user with is_staff_caller (§11.8)"
    );
    assert!(
        !src.contains(".filter(|s| !s.is_owner())"),
        "the fixed owner filter on staff_user is gone (§11.8)"
    );
}
