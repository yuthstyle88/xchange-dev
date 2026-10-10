//! #1149 — `Need::AnyOf`: one route-table row that admits an actor holding **any** of several
//! permissions, refused exactly as a single-permission row is refused when none is held.
//! Written before the implementation by the ticket author (claude/14cc24bd, on the lane controller's
//! order, 2026-10-10). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Why: `staff_holds` (#573, design `docs/design/xchange-v3-wave3-staff.md` §3.1 / B9) is for
//! `CustomerHoldPlace` **or** `VerificationReview`, and `Need` held one permission
//! (`policy.rs:505-512`).
//!
//! No database is used here; nothing here skips.
//!
//! Signatures this test assumes ([required]):
//! - `Need::AnyOf(&'static [Permission])`, a new variant of
//!   `xc_auth::domain::policy::entities::policy::Need` (still `Debug + Clone + Copy + PartialEq +
//!   Eq`, so an `AreaRow` can name it).
//! - `Policy::authorize_with(areas: &[&[AreaRow]], actor: Option<&access::Actor>, path: EntryPath,
//!   action: &Action, named: Option<&BranchId>, now: DateTime<Utc>) -> Result<Grant, PolicyError>`:
//!   `authorize` over the given area rows, as `need_with` is `need` over them. `Policy::authorize`
//!   is `authorize_with(AREA_ROWS, …)` and answers exactly as before for every existing row.
//! - `AnyOf(ps)` grants when **any** `p` in `ps` passes `Authorizer::check` for the target (the
//!   same check `Permission(p)` makes), on the branch that permission grants; when none does, it
//!   answers the refusal a `Permission` row answers on that path (the path's wording, `OutOfScope`
//!   for a branch out of scope, the path's sign-in sentence for no actor). An empty `ps` is refused.

mod policy_common;

use policy_common::*;
use xc_auth::domain::policy::areas::{AREA_ROWS, AreaRow};

const HOLDS: &[Permission] = &[
    Permission::CustomerHoldPlace,
    Permission::VerificationReview,
];

/// Test-only rows on two paths whose refusals differ in wording (StaffRpc: `RoleNotAllowed`; Rpc:
/// `RoleNotAllowedHere`). None of these actions has a row in the table.
const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "zzAnyOfHolds",
        need: Need::AnyOf(HOLDS),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "zzOnlyHoldPlace",
        need: Need::Permission(Permission::CustomerHoldPlace),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "zzOnlyReview",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "zzAnyOfHolds",
        need: Need::AnyOf(HOLDS),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "zzOnlyHoldPlace",
        need: Need::Permission(Permission::CustomerHoldPlace),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "zzAnyOfNothing",
        need: Need::AnyOf(&[]),
    },
];

fn ask(
    who: Option<&Actor>,
    on: EntryPath,
    name: &str,
    named: Option<&str>,
) -> Result<Grant, PolicyError> {
    Policy::authorize_with(
        &[ROWS],
        who,
        on,
        &action(name),
        named.map(branch).as_ref(),
        now(),
    )
}

/// Only `CustomerHoldPlace` of the two (TELLER, `role_permissions.rs`), at `own`.
fn hold_placer(own: &str) -> Actor {
    staff(Role::Teller, own)
}

/// Only `VerificationReview` of the two (VERIFICATION_REVIEWER), at `own`.
fn reviewer(own: &str) -> Actor {
    staff(Role::VerificationReviewer, own)
}

/// Neither of the two (RATE_MANAGER), at `own`.
fn neither(own: &str) -> Actor {
    staff(Role::RateManager, own)
}

#[test]
fn the_fixture_roles_hold_what_the_tests_say() {
    // the premise of every test below, read from the role table, not assumed
    let has = |r: Role, p: Permission| RolePermissions::of(r).contains(&p);
    assert!(has(Role::Teller, Permission::CustomerHoldPlace));
    assert!(!has(Role::Teller, Permission::VerificationReview));
    assert!(has(
        Role::VerificationReviewer,
        Permission::VerificationReview
    ));
    assert!(!has(
        Role::VerificationReviewer,
        Permission::CustomerHoldPlace
    ));
    assert!(!has(Role::RateManager, Permission::CustomerHoldPlace));
    assert!(!has(Role::RateManager, Permission::VerificationReview));
}

#[test]
fn any_of_grants_an_actor_who_holds_only_the_first_permission() {
    let g = ask(
        Some(&hold_placer("B1")),
        EntryPath::StaffRpc,
        "zzAnyOfHolds",
        Some("B1"),
    )
    .expect("CustomerHoldPlace alone is enough");
    assert_eq!(g.branch, branch("B1"));
}

#[test]
fn any_of_grants_an_actor_who_holds_only_the_second_permission() {
    let g = ask(
        Some(&reviewer("B1")),
        EntryPath::StaffRpc,
        "zzAnyOfHolds",
        Some("B1"),
    )
    .expect("VerificationReview alone is enough");
    assert_eq!(g.branch, branch("B1"));
}

#[test]
fn any_of_refuses_an_actor_with_neither_exactly_as_a_single_permission_row_does() {
    for on in [EntryPath::StaffRpc, EntryPath::Rpc] {
        let any = ask(Some(&neither("B1")), on, "zzAnyOfHolds", Some("B1"));
        let one = ask(Some(&neither("B1")), on, "zzOnlyHoldPlace", Some("B1"));
        assert!(any.is_err(), "{on:?}: neither permission is held: {any:?}");
        assert_eq!(
            any, one,
            "{on:?}: the same refusal, in the path's own words"
        );
    }
    // and the two paths' words differ, so the comparison above is not vacuous
    assert_ne!(
        ask(
            Some(&neither("B1")),
            EntryPath::StaffRpc,
            "zzAnyOfHolds",
            Some("B1")
        ),
        ask(
            Some(&neither("B1")),
            EntryPath::Rpc,
            "zzAnyOfHolds",
            Some("B1")
        )
    );
}

#[test]
fn any_of_without_an_actor_answers_the_paths_sign_in_sentence() {
    for on in [EntryPath::StaffRpc, EntryPath::Rpc] {
        assert_eq!(
            ask(None, on, "zzAnyOfHolds", Some("B1")),
            ask(None, on, "zzOnlyHoldPlace", Some("B1")),
            "{on:?}"
        );
    }
}

#[test]
fn a_permission_held_at_another_branch_is_out_of_scope_as_for_a_single_permission() {
    let any = ask(
        Some(&hold_placer("B1")),
        EntryPath::StaffRpc,
        "zzAnyOfHolds",
        Some("B2"),
    );
    let one = ask(
        Some(&hold_placer("B1")),
        EntryPath::StaffRpc,
        "zzOnlyHoldPlace",
        Some("B2"),
    );
    assert_eq!(any, Err(PolicyError::OutOfScope));
    assert_eq!(any, one);
    // the same when the permission held elsewhere is the second of the two: the out-of-scope
    // refusal wins over the role refusal of the other permission, whichever order they come in
    let any = ask(
        Some(&reviewer("B1")),
        EntryPath::StaffRpc,
        "zzAnyOfHolds",
        Some("B2"),
    );
    let one = ask(
        Some(&reviewer("B1")),
        EntryPath::StaffRpc,
        "zzOnlyReview",
        Some("B2"),
    );
    assert_eq!(any, Err(PolicyError::OutOfScope));
    assert_eq!(any, one);
}

#[test]
fn each_permission_keeps_its_own_branch_scope() {
    // CustomerHoldPlace at B1, VerificationReview at B2
    let both = user(
        "u-both",
        vec![
            row(Role::Teller, Some("B1")),
            row(Role::VerificationReviewer, Some("B2")),
        ],
    );
    let on = EntryPath::StaffRpc;
    assert_eq!(
        ask(Some(&both), on, "zzAnyOfHolds", Some("B1"))
            .expect("B1 through CustomerHoldPlace")
            .branch,
        branch("B1")
    );
    assert_eq!(
        ask(Some(&both), on, "zzAnyOfHolds", Some("B2"))
            .expect("B2 through VerificationReview")
            .branch,
        branch("B2")
    );
    assert_eq!(
        ask(Some(&both), on, "zzAnyOfHolds", Some("B3")),
        Err(PolicyError::OutOfScope),
        "neither permission reaches B3"
    );
    // a single row still admits only its own permission's branch
    assert_eq!(
        ask(Some(&both), on, "zzOnlyReview", Some("B1")),
        Err(PolicyError::OutOfScope)
    );
}

#[test]
fn an_empty_any_of_admits_nobody() {
    for who in [hold_placer("B1"), reviewer("B1"), neither("B1"), owner()] {
        assert!(
            ask(
                Some(&who),
                EntryPath::StaffRpc,
                "zzAnyOfNothing",
                Some("B1")
            )
            .is_err(),
            "an empty AnyOf is refused for {:?}",
            who.roles
        );
    }
}

#[test]
fn authorize_is_authorize_with_the_real_area_rows() {
    // every path × a spread of actions and actors: `authorize` answers what `authorize_with`
    // answers over `AREA_ROWS`
    let actors = [
        None,
        Some(owner()),
        Some(hold_placer("B1")),
        Some(reviewer("B1")),
        Some(neither("B1")),
        Some(staff(Role::BranchManager, "B1")),
    ];
    let names = [
        "saveTransaction",
        "adminSnapshot",
        "adminSaveRate",
        "approvalDecide",
        "documents",
        "printerOnline",
        "rates",
        "zzNoSuchAction",
    ];
    for on in PATHS {
        for name in names {
            for who in &actors {
                for named in [None, Some("B1"), Some("B2")] {
                    let b = named.map(branch);
                    assert_eq!(
                        Policy::authorize(who.as_ref(), on, &action(name), b.as_ref(), now()),
                        Policy::authorize_with(
                            AREA_ROWS,
                            who.as_ref(),
                            on,
                            &action(name),
                            b.as_ref(),
                            now()
                        ),
                        "{on:?} {name} {named:?}"
                    );
                }
            }
        }
    }
}
