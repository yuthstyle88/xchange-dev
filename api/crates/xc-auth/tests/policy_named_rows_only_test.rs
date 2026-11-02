//! Up-front tests for #5682 (F-3 of #5558, design `docs/design/org-wide-actor-branch.md` §15.12): the paths
//! `Devices`, `OwnerRoute` and `UserRoute` lose their catch-all defaults in `Policy::need_table`, and their named
//! rows move into area files, so a name those paths do not spell has no row and is refused.
//!
//! Written by the ticket author (claude/e2926a49) before the implementation. Do not edit, delete or weaken; add
//! tests in a file of your own. No test here needs a database.

mod policy_common;

use policy_common::*;
use xc_auth::domain::policy::areas::{AREA_ROWS, RowScope, duplicates, shadowed};

/// Every named row of the three paths, with the permission it needs on `main` `61382f2f` (`policy.rs:107-124`).
const NAMED: &[(EntryPath, &str, Permission)] = &[
    (EntryPath::Devices, "setPrinter", Permission::SystemSettings),
    (EntryPath::Devices, "setDrawer", Permission::SystemSettings),
    (EntryPath::Devices, "queue", Permission::DeviceQueue),
    (EntryPath::Devices, "process", Permission::DeviceProcess),
    (EntryPath::Devices, "retry", Permission::DeviceProcess),
    (
        EntryPath::Devices,
        "drawerObservation",
        Permission::DeviceProcess,
    ),
    (
        EntryPath::Devices,
        "printerOnline",
        Permission::DevicePrinterOnline,
    ),
    // the device views' `INHERITS` (`views/devices.rs:49`, `device_events.rs:44`, `device_jobs.rs:43`,
    // `device_transactions.rs:40`) and the `snapshot` arm of `http/devices.rs:106`
    (EntryPath::Devices, "snapshot", Permission::SignedIn),
    (
        EntryPath::OwnerRoute,
        "/api/reference-rates",
        Permission::ReferenceRatesRead,
    ),
    (
        EntryPath::OwnerRoute,
        "queueTicketTake",
        Permission::CustomerSelf,
    ),
    (EntryPath::UserRoute, "documents", Permission::DocumentsList),
];

const PATHS_5682: [EntryPath; 3] = [
    EntryPath::Devices,
    EntryPath::OwnerRoute,
    EntryPath::UserRoute,
];

/// Names no route asks on these paths: the catch-alls answered them until #5682.
const UNKNOWN: &[&str] = &[
    "nope",
    "",
    "employees",
    "list",
    "receipt",
    "admin",
    "desk",
    "zzNoSuchAction",
];

#[test]
fn f3_row1_each_named_row_keeps_its_permission() {
    // Restated for #6004 (user-management-rbac.md §11.11): setPrinter and setDrawer map to BranchesManage.
    for &(on, name, p) in NAMED {
        let p = match (on, name) {
            (EntryPath::Devices, "setPrinter" | "setDrawer") => Permission::BranchesManage,
            _ => p,
        };
        assert_eq!(
            Policy::need(on, &action(name)),
            Some(Need::Permission(p)),
            "{on:?} {name:?}"
        );
    }
}

#[test]
fn f3_row2_each_named_row_is_one_request_scoped_area_row() {
    // Restated for #6004 (user-management-rbac.md §11.11): setPrinter and setDrawer map to BranchesManage.
    for &(on, name, p) in NAMED {
        let p = match (on, name) {
            (EntryPath::Devices, "setPrinter" | "setDrawer") => Permission::BranchesManage,
            _ => p,
        };
        let rows: Vec<_> = AREA_ROWS
            .iter()
            .flat_map(|area| area.iter())
            .filter(|row| row.path == on && row.action == name)
            .collect();
        assert_eq!(rows.len(), 1, "{on:?} {name:?}: one area row");
        assert_eq!(rows[0].need, Need::Permission(p), "{on:?} {name:?}");
        assert_eq!(rows[0].scope, RowScope::Request, "{on:?} {name:?}");
    }
}

#[test]
fn f3_row3_the_table_answers_none_of_the_three_paths() {
    for on in PATHS_5682 {
        let names = NAMED
            .iter()
            .filter(|(p, _, _)| *p == on)
            .map(|(_, n, _)| *n)
            .chain(UNKNOWN.iter().copied());
        for name in names {
            assert_eq!(
                Policy::need_table(on, &action(name)),
                None,
                "need_table({on:?}, {name:?})"
            );
        }
    }
}

#[test]
fn f3_row4_a_name_the_path_does_not_spell_has_no_row() {
    for on in PATHS_5682 {
        for name in UNKNOWN {
            assert_eq!(Policy::need(on, &action(name)), None, "{on:?} {name:?}");
        }
    }
}

#[test]
fn f3_row5_a_signed_in_caller_naming_an_unknown_action_hears_invalid_request() {
    let callers = [
        owner(),
        staff(Role::Teller, "B2"),
        staff(Role::Admin, "B2"),
        staff(Role::BranchManager, "B2"),
    ];
    for on in PATHS_5682 {
        for who in &callers {
            for name in UNKNOWN {
                assert_eq!(
                    ask(Some(who), on, name),
                    Err(PolicyError::InvalidRequest),
                    "{on:?} {name:?}"
                );
            }
        }
    }
}

#[test]
fn f3_row6_nobody_signed_in_still_hears_the_sign_in_sentence() {
    for on in PATHS_5682 {
        for name in UNKNOWN.iter().chain(&["snapshot", "documents"]) {
            assert_eq!(
                ask(None, on, name),
                Err(PolicyError::SignIn),
                "{on:?} {name:?}"
            );
        }
    }
}

#[test]
fn f3_row7_the_moved_rows_are_neither_shadowed_nor_duplicated() {
    assert_eq!(shadowed(AREA_ROWS), vec![]);
    assert_eq!(duplicates(AREA_ROWS), vec![]);
}
