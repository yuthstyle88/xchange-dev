//! Who may use `POST /api/staff/bookings`, the desk's bookings route.
//!
//! Provided with ticket T15-d (#410; design `docs/design/xchange-v3.md` §5.6.4
//! *`EntryPath::StaffBookings`*, §5.6.8). Written before the implementation. Do not edit, delete
//! or weaken.
//!
//! v2 has no answer to record here: its staff door allowed only `login`, `logout`, `password`,
//! `me`, `rpc`, `upload` (10-06 bundle `server.mjs:7493`, `:7498`), so every call to
//! `/api/staff/bookings` was a bad request. The owner decided on 2026-10-08 (PR 395
//! issuecomment-6050139400) that the counter roles — FRONT, BACKEND, MANAGER, the roles the kiosk
//! staff actions and the passport drawer already let in — may `list`, `get`, `create`, `confirm`
//! and `cancel` there, and that `member`, `settings`, `registrations` and `approveRegistration`
//! stay the owner's. The sentences are existing `PolicyError`s: no new one.
//!
//! Not pinned here, on purpose (open on #410): which sentence a role outside the counter gets for
//! an owner action or an unknown name. Those calls are only required to be refused.
//!
//! Moved by the author of OR-4 (#550, `docs/design/blueprint/org-roles-branches.md` §4.2, §4.3)
//! onto the Blueprint's users. Three answers change:
//! - C9 (§03 B02, user TELLER): the counter actions are BookingsDesk, TELLER's alone; BRANCH_MANAGER
//!   (ex-BACKEND, ex-MANAGER) is refused like any other role outside the counter;
//! - C1 (§07, one User): the owner's user is answered by its roles, EXECUTIVE + ADMIN, neither of
//!   which holds BookingsDesk, so it hears what a role outside the counter hears;
//! - D-4: an employee naming a branch outside their scope is refused, no longer moved to their own.
//!
//! v2's owner bookings route and its BookingsAdmin rows went with v2's booking (BK-11 #1413).

mod policy_common;

use policy_common::*;

const B2: &str = "B2";

const COUNTER_ACTIONS: [&str; 5] = ["list", "get", "create", "confirm", "cancel"];
const OWNER_ACTIONS: [&str; 4] = ["member", "settings", "registrations", "approveRegistration"];
/// Names that are no booking action: v2's spellings are exact.
const UNKNOWN: [&str; 9] = [
    "",
    "nope",
    "List",
    "CREATE",
    "cancel ",
    "approveregistration",
    "use",
    "delete",
    "adminSnapshot",
];

const COUNTER_ROLES: [Role; 1] = [Role::Teller];
/// C9: BRANCH_MANAGER is outside the counter now.
const OTHER_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Finance,
    Role::Marketing,
    Role::Customer,
    Role::VerificationReviewer,
];

fn every_name() -> Vec<&'static str> {
    let mut all = COUNTER_ACTIONS.to_vec();
    all.extend(OWNER_ACTIONS);
    all.extend(UNKNOWN);
    all
}

#[test]
fn the_path_has_a_name() {
    assert_eq!(path_name(EntryPath::StaffBookings), "StaffBookings");
    // a path of its own, not one of v2's recorded paths
    assert!(!PATHS.contains(&EntryPath::StaffBookings));
}

#[test]
fn nobody_must_sign_in_as_an_employee_and_the_owners_user_is_outside_the_counter() {
    for name in every_name() {
        assert_eq!(
            said(ask(None, EntryPath::StaffBookings, name)),
            (EMPLOYEE_AUTH.to_owned(), 401),
            "nobody, {name:?}"
        );
        // C1: was EMPLOYEE_AUTH_REQUIRED (the owner was no employee); now its roles answer
        assert_eq!(
            said(ask(Some(&owner()), EntryPath::StaffBookings, name)),
            (ROLE_NOT_ALLOWED.to_owned(), 400),
            "the owner's user, {name:?}"
        );
    }
}

#[test]
fn the_counter_roles_list_get_create_confirm_and_cancel() {
    for r in COUNTER_ROLES {
        for name in COUNTER_ACTIONS {
            assert_eq!(
                ask(Some(&staff(r, B2)), EntryPath::StaffBookings, name),
                Ok(()),
                "{} {name}",
                r.as_str()
            );
        }
    }
}

#[test]
fn the_owner_actions_are_the_owners_alone() {
    for r in COUNTER_ROLES {
        for name in OWNER_ACTIONS {
            assert_eq!(
                said(ask(Some(&staff(r, B2)), EntryPath::StaffBookings, name)),
                (OWNER_ONLY.to_owned(), 403),
                "{} {name}",
                r.as_str()
            );
        }
    }
}

#[test]
fn the_other_roles_are_refused() {
    for r in OTHER_ROLES {
        for name in COUNTER_ACTIONS {
            assert_eq!(
                said(ask(Some(&staff(r, B2)), EntryPath::StaffBookings, name)),
                (ROLE_NOT_ALLOWED.to_owned(), 400),
                "{} {name}",
                r.as_str()
            );
        }
    }
}

#[test]
fn no_employee_reaches_an_owner_action() {
    for r in BLUEPRINT {
        for name in OWNER_ACTIONS.into_iter().chain(UNKNOWN) {
            assert!(
                ask(Some(&staff(r, B2)), EntryPath::StaffBookings, name).is_err(),
                "{} {name:?} must be refused",
                r.as_str()
            );
        }
    }
}

#[test]
fn a_name_the_bookings_do_not_know_is_a_bad_request() {
    for r in COUNTER_ROLES {
        for name in UNKNOWN {
            assert_eq!(
                said(ask(Some(&staff(r, B2)), EntryPath::StaffBookings, name)),
                (INVALID_REQUEST.to_owned(), 400),
                "{} {name:?}",
                r.as_str()
            );
        }
    }
}

#[test]
fn staff_bookings_run_on_the_employees_own_branch() {
    // §5.6.4 row 5, under D-4: an employee works on their own branch, and naming another is refused
    for r in COUNTER_ROLES {
        let who = staff(r, B2);
        assert_eq!(
            said(ask_on(
                Some(&who),
                EntryPath::StaffBookings,
                "list",
                Some("MAIN")
            )),
            (OUT_OF_SCOPE.to_owned(), 403)
        );
        assert_eq!(
            ask_on(Some(&who), EntryPath::StaffBookings, "list", Some(B2)),
            Ok(branch(B2))
        );
        assert_eq!(
            ask_on(Some(&who), EntryPath::StaffBookings, "list", None),
            Ok(branch(B2))
        );
    }
}
