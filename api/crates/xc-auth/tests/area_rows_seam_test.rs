//! Up-front tests for PL-0b #1014 (`docs/design/blueprint/broker-seam.md` §11.1): how
//! `Policy::need` consults the area rows, and the two checks `area_rows_test.rs` runs on the real
//! `AREA_ROWS`.
//!
//! Written by the ticket author before the implementation. Do not edit, delete or weaken.
//! No test here needs a database.
//!
//! `TODAY` is `Policy::need` as it answered on `main` `04a40bd4`, before PL-0b, for every entry path
//! × every action name the match spells (plus `opsProbe` for the `starts_with("ops")` arm and
//! `zzNoSuchAction` for the catch-alls). PL-0b moves the match behind `need_table` unchanged, so
//! `need_table`, and `need` with no area files, must answer exactly this.
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7 removes `adminSaveRate`
//! from the match (`docs/design/blueprint/rates.md` §9), so its Rpc and StaffRpc rows of `TODAY`
//! answer `None` (no row) instead of RatesDraft. On every other path it falls to the same
//! catch-all as before, so those rows are unchanged; so is every other row.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c): CA-10 retires the 13 `ops*`
//! actions and the direct shift actions `adminOpenShift` / `adminCloseShift` / `adminMoveStock`
//! with their policy rows (`docs/design/blueprint/cash-shift-transfer.md` §6). So on `Rpc` and
//! `StaffRpc` those names and the `starts_with("ops")` arm (`ops`, `opsProbe`) answer `None` (no
//! row). `EntryPath::Custody` and `Need::Custody` stay as types (other suites name them; no route
//! reaches them), so the `Custody` rows are unchanged. On every other path these names fall to the
//! same catch-all as before; so is every other row.
//!
//! Re-expected for #5558 by its ticket author (claude/2ca5874b; `docs/design/org-wide-actor-branch.md`
//! §12): the per-action rows of `Rpc`, `StaffRpc`, `Kiosk` and `PaymentMethods` move from `need_table` into area
//! files, so `TODAY` is now what `Policy::need` (area rows, then the table) answers, not `need_table` alone, and
//! with no area rows `need` is the table alone. Every row of `TODAY` keeps its answer.
//!
//! Re-expected for #5682 by its ticket author (claude/e2926a49; `docs/design/org-wide-actor-branch.md` §15.12):
//! `EntryPath::StaffDesk`, `EntryPath::Custody` and `Need::Custody` are deleted, so their rows leave `TODAY`. The
//! catch-all defaults of `Devices`, `OwnerRoute` and `UserRoute` are deleted: a name those paths do not spell answers
//! `None` (no row). Their named rows keep their answers.

use xc_auth::domain::access::Permission;
use xc_auth::domain::policy::areas::{AREA_ROWS, AreaRow, RowScope, duplicates, shadowed};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

const NEW_ROW: AreaRow = AreaRow {
    path: EntryPath::Rpc,
    action: "attachReferral",
    need: Need::Permission(Permission::ExchangeTransact),
    scope: RowScope::Request,
};

#[test]
fn an_area_row_answers_its_own_path_and_action() {
    let area: &[AreaRow] = &[NEW_ROW];
    assert_eq!(
        Policy::need_with(&[area], EntryPath::Rpc, &Action::new("attachReferral")),
        Some(Need::Permission(Permission::ExchangeTransact))
    );
}

#[test]
fn an_area_row_answers_nothing_else() {
    let area: &[AreaRow] = &[NEW_ROW];
    // Same action, another path: the table's answer (none of the table's rows name it).
    assert_eq!(
        Policy::need_with(&[area], EntryPath::StaffRpc, &Action::new("attachReferral")),
        Policy::need_table(EntryPath::StaffRpc, &Action::new("attachReferral"))
    );
    // Another path the table answers (#5558: the Rpc rows live in area files now): as before.
    assert_eq!(
        Policy::need_with(&[area], EntryPath::Upload, &Action::new("upload")),
        Some(Need::Permission(Permission::ProofUpload))
    );
    assert_eq!(
        Policy::need_with(&[area], EntryPath::Rpc, &Action::new("zzNoSuchAction")),
        None
    );
}

#[test]
fn without_the_area_file_the_row_is_gone() {
    assert_eq!(
        Policy::need_with(&[], EntryPath::Rpc, &Action::new("attachReferral")),
        None
    );
}

#[test]
fn rows_in_a_second_area_file_are_found_too() {
    let first: &[AreaRow] = &[NEW_ROW];
    let second: &[AreaRow] = &[AreaRow {
        path: EntryPath::UserRoute,
        action: "myReceipt",
        need: Need::Permission(Permission::CustomerSelf),
        scope: RowScope::Request,
    }];
    assert_eq!(
        Policy::need_with(
            &[first, second],
            EntryPath::UserRoute,
            &Action::new("myReceipt")
        ),
        Some(Need::Permission(Permission::CustomerSelf))
    );
}

#[test]
fn need_table_is_todays_route_table_unchanged() {
    // #5558: `TODAY` is what `Policy::need` answers (area rows, then the table).
    // Restated for #6004 (user-management-rbac.md §11.11): DeviceSetup, setPrinter, and setDrawer map to BranchesManage.
    for &(path, action, want) in TODAY {
        let want = match (path, action) {
            (EntryPath::DeviceSetup, _) | (EntryPath::Devices, "setDrawer" | "setPrinter") => {
                Some(Need::Permission(Permission::BranchesManage))
            }
            _ => want,
        };
        assert_eq!(
            Policy::need(path, &Action::new(action)),
            want,
            "need({path:?}, {action:?})"
        );
    }
}

#[test]
fn with_no_area_rows_need_answers_exactly_as_today() {
    // #5558: with no area rows, `need` is the table alone.
    for &(path, action, _) in TODAY {
        assert_eq!(
            Policy::need_with(&[], path, &Action::new(action)),
            Policy::need_table(path, &Action::new(action)),
            "need_with([], {path:?}, {action:?})"
        );
        assert_eq!(
            Policy::need(path, &Action::new(action)),
            Policy::need_with(AREA_ROWS, path, &Action::new(action)),
            "need({path:?}, {action:?}) must be need_with(AREA_ROWS, …)"
        );
    }
}

#[test]
fn duplicates_finds_a_pair_named_in_two_area_files_once() {
    let a: &[AreaRow] = &[NEW_ROW];
    let b: &[AreaRow] = &[AreaRow {
        need: Need::Public,
        ..NEW_ROW
    }];
    let c: &[AreaRow] = &[NEW_ROW];
    assert_eq!(
        duplicates(&[a, b]),
        vec![(EntryPath::Rpc, "attachReferral")]
    );
    assert_eq!(
        duplicates(&[a, b, c]),
        vec![(EntryPath::Rpc, "attachReferral")]
    );
}

#[test]
fn duplicates_finds_a_pair_named_twice_in_one_file() {
    let a: &[AreaRow] = &[NEW_ROW, NEW_ROW];
    assert_eq!(duplicates(&[a]), vec![(EntryPath::Rpc, "attachReferral")]);
}

#[test]
fn the_same_action_on_two_paths_is_not_a_duplicate() {
    let a: &[AreaRow] = &[NEW_ROW];
    let b: &[AreaRow] = &[AreaRow {
        path: EntryPath::StaffRpc,
        ..NEW_ROW
    }];
    assert_eq!(duplicates(&[a, b]), vec![]);
    assert_eq!(duplicates(&[]), vec![]);
}

#[test]
fn shadowed_finds_an_area_row_the_table_already_answers() {
    let a: &[AreaRow] = &[
        NEW_ROW,
        AreaRow {
            path: EntryPath::DeviceSetup,
            action: "add",
            need: Need::Public,
            scope: RowScope::Request,
        },
    ];
    // #5558: the table answers whole paths only now, so the example names one of them.
    assert_eq!(shadowed(&[a]), vec![(EntryPath::DeviceSetup, "add")]);
}

#[test]
fn shadowed_counts_a_catch_all_answer_as_answered() {
    // Every action on `Upload` is answered by the table (`EntryPath::Upload => Some(..)`), so an
    // area row there would silently override it.
    let a: &[AreaRow] = &[AreaRow {
        path: EntryPath::Upload,
        action: "anything",
        need: Need::Public,
        scope: RowScope::Request,
    }];
    assert_eq!(shadowed(&[a]), vec![(EntryPath::Upload, "anything")]);
}

#[test]
fn shadowed_lets_a_new_row_through() {
    let a: &[AreaRow] = &[NEW_ROW];
    assert_eq!(shadowed(&[a]), vec![]);
    assert_eq!(shadowed(&[]), vec![]);
}

#[rustfmt::skip]
const TODAY: &[(EntryPath, &str, Option<Need>)] = &[
    (EntryPath::Rpc, "accept", None),
    (EntryPath::Rpc, "adminAbandonRequest", Some(Need::Permission(Permission::RequestAbandon))),
    (EntryPath::Rpc, "adminCloseShift", None),
    (EntryPath::Rpc, "adminCreateBranch", Some(Need::Permission(Permission::BranchesManage))),
    (EntryPath::Rpc, "adminDeskConfig", Some(Need::Permission(Permission::CompatRead))),
    (EntryPath::Rpc, "adminMoveStock", None),
    (EntryPath::Rpc, "adminOpenShift", None),
    (EntryPath::Rpc, "adminSaveRate", None), // RE-7 #604 (rates.md §9): was RatesDraft
    (EntryPath::Rpc, "adminSnapshot", Some(Need::Permission(Permission::CompatRead))),
    (EntryPath::Rpc, "approvalDecide", Some(Need::Permission(Permission::ApprovalsDecide))),
    (EntryPath::Rpc, "approvalRead", Some(Need::Permission(Permission::ApprovalsDecide))),
    (EntryPath::Rpc, "approveRegistration", None),
    (EntryPath::Rpc, "cancel", None),
    (EntryPath::Rpc, "configure", None),
    (EntryPath::Rpc, "confirm", None),
    (EntryPath::Rpc, "consentPolicy", Some(Need::Permission(Permission::ConsentPolicyManage))),
    (EntryPath::Rpc, "create", None),
    (EntryPath::Rpc, "customerReceipt", None),
    (EntryPath::Rpc, "documents", None),
    (EntryPath::Rpc, "drawerObservation", None),
    (EntryPath::Rpc, "get", None),
    (EntryPath::Rpc, "getConnectionStatus", Some(Need::Permission(Permission::CompatRead))),
    (EntryPath::Rpc, "identityChecked", None),
    (EntryPath::Rpc, "list", None),
    (EntryPath::Rpc, "manage", None),
    (EntryPath::Rpc, "member", None),
    (EntryPath::Rpc, "notifications", None),
    (EntryPath::Rpc, "ops", None),
    (EntryPath::Rpc, "opsProbe", None),
    (EntryPath::Rpc, "printerOnline", None),
    (EntryPath::Rpc, "process", None),
    (EntryPath::Rpc, "publishReceipt", None),
    (EntryPath::Rpc, "queue", None),
    (EntryPath::Rpc, "quote", None),
    (EntryPath::Rpc, "rateDraft", Some(Need::Permission(Permission::RatesDraft))),
    (EntryPath::Rpc, "ratePolicySave", Some(Need::Permission(Permission::RatePolicySave))),
    (EntryPath::Rpc, "rates", None),
    (EntryPath::Rpc, "registrations", None),
    (EntryPath::Rpc, "reserveCustomer", Some(Need::Permission(Permission::ExchangeTransact))),
    (EntryPath::Rpc, "retry", None),
    (EntryPath::Rpc, "reversalRequest", Some(Need::Permission(Permission::ReversalRequest))),
    (EntryPath::Rpc, "save", None),
    (EntryPath::Rpc, "saveTransaction", Some(Need::Permission(Permission::ExchangeTransact))),
    (EntryPath::Rpc, "setDrawer", None),
    (EntryPath::Rpc, "setPrinter", None),
    (EntryPath::Rpc, "settings", None),
    (EntryPath::Rpc, "status", None),
    (EntryPath::Rpc, "zzNoSuchAction", None),
    (EntryPath::StaffRpc, "accept", None),
    (EntryPath::StaffRpc, "adminAbandonRequest", None),
    (EntryPath::StaffRpc, "adminCloseShift", None),
    (EntryPath::StaffRpc, "adminCreateBranch", None),
    (EntryPath::StaffRpc, "adminDeskConfig", Some(Need::Permission(Permission::ExchangeTransact))),
    (EntryPath::StaffRpc, "adminMoveStock", None),
    (EntryPath::StaffRpc, "adminOpenShift", None),
    (EntryPath::StaffRpc, "adminSaveRate", None), // RE-7 #604 (rates.md §9): was RatesDraft
    (EntryPath::StaffRpc, "adminSnapshot", Some(Need::Permission(Permission::CompatRead))),
    (EntryPath::StaffRpc, "approvalDecide", None),
    (EntryPath::StaffRpc, "approvalRead", None),
    (EntryPath::StaffRpc, "approveRegistration", None),
    (EntryPath::StaffRpc, "cancel", None),
    (EntryPath::StaffRpc, "configure", None),
    (EntryPath::StaffRpc, "confirm", None),
    (EntryPath::StaffRpc, "consentPolicy", None),
    (EntryPath::StaffRpc, "create", None),
    (EntryPath::StaffRpc, "customerReceipt", None),
    (EntryPath::StaffRpc, "documents", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffRpc, "drawerObservation", None),
    (EntryPath::StaffRpc, "get", None),
    (EntryPath::StaffRpc, "getConnectionStatus", Some(Need::Permission(Permission::CompatRead))),
    (EntryPath::StaffRpc, "identityChecked", None),
    (EntryPath::StaffRpc, "list", None),
    (EntryPath::StaffRpc, "manage", None),
    (EntryPath::StaffRpc, "member", None),
    (EntryPath::StaffRpc, "notifications", None),
    (EntryPath::StaffRpc, "ops", None),
    (EntryPath::StaffRpc, "opsProbe", None),
    (EntryPath::StaffRpc, "printerOnline", None),
    (EntryPath::StaffRpc, "process", None),
    (EntryPath::StaffRpc, "publishReceipt", None),
    (EntryPath::StaffRpc, "queue", None),
    (EntryPath::StaffRpc, "quote", None),
    (EntryPath::StaffRpc, "rateDraft", Some(Need::Permission(Permission::RatesDraft))),
    (EntryPath::StaffRpc, "ratePolicySave", None),
    (EntryPath::StaffRpc, "rates", None),
    (EntryPath::StaffRpc, "registrations", None),
    (EntryPath::StaffRpc, "reserveCustomer", Some(Need::Permission(Permission::ExchangeTransact))),
    (EntryPath::StaffRpc, "retry", None),
    (EntryPath::StaffRpc, "reversalRequest", None),
    (EntryPath::StaffRpc, "save", None),
    (EntryPath::StaffRpc, "saveTransaction", Some(Need::Permission(Permission::ExchangeTransact))),
    (EntryPath::StaffRpc, "setDrawer", None),
    (EntryPath::StaffRpc, "setPrinter", None),
    (EntryPath::StaffRpc, "settings", None),
    (EntryPath::StaffRpc, "status", None),
    (EntryPath::StaffRpc, "zzNoSuchAction", None),
    (EntryPath::Upload, "accept", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminAbandonRequest", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminCloseShift", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminCreateBranch", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminDeskConfig", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminMoveStock", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminOpenShift", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminSaveRate", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "adminSnapshot", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "approvalDecide", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "approvalRead", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "approveRegistration", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "cancel", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "configure", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "confirm", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "consentPolicy", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "create", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "customerReceipt", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "documents", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "drawerObservation", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "get", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "getConnectionStatus", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "identityChecked", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "list", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "manage", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "member", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "notifications", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "ops", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "opsProbe", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "printerOnline", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "process", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "publishReceipt", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "queue", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "quote", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "rateDraft", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "ratePolicySave", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "rates", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "registrations", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "reserveCustomer", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "retry", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "reversalRequest", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "save", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "saveTransaction", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "setDrawer", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "setPrinter", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "settings", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "status", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::Upload, "zzNoSuchAction", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "accept", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminAbandonRequest", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminCloseShift", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminCreateBranch", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminDeskConfig", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminMoveStock", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminOpenShift", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminSaveRate", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "adminSnapshot", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "approvalDecide", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "approvalRead", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "approveRegistration", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "cancel", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "configure", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "confirm", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "consentPolicy", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "create", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "customerReceipt", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "documents", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "drawerObservation", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "get", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "getConnectionStatus", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "identityChecked", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "list", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "manage", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "member", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "notifications", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "ops", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "opsProbe", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "printerOnline", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "process", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "publishReceipt", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "queue", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "quote", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "rateDraft", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "ratePolicySave", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "rates", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "registrations", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "reserveCustomer", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "retry", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "reversalRequest", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "save", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "saveTransaction", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "setDrawer", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "setPrinter", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "settings", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "status", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::StaffUpload, "zzNoSuchAction", Some(Need::Permission(Permission::ProofUpload))),
    (EntryPath::File, "accept", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminAbandonRequest", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminCloseShift", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminCreateBranch", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminDeskConfig", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminMoveStock", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminOpenShift", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminSaveRate", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "adminSnapshot", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "approvalDecide", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "approvalRead", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "approveRegistration", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "cancel", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "configure", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "confirm", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "consentPolicy", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "create", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "customerReceipt", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "documents", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "drawerObservation", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "get", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "getConnectionStatus", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "identityChecked", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "list", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "manage", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "member", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "notifications", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "ops", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "opsProbe", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "printerOnline", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "process", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "publishReceipt", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "queue", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "quote", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "rateDraft", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "ratePolicySave", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "rates", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "registrations", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "reserveCustomer", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "retry", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "reversalRequest", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "save", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "saveTransaction", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "setDrawer", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "setPrinter", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "settings", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "status", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::File, "zzNoSuchAction", Some(Need::Permission(Permission::FileRead))),
    (EntryPath::StaffFile, "accept", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminAbandonRequest", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminCloseShift", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminCreateBranch", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminDeskConfig", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminMoveStock", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminOpenShift", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminSaveRate", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "adminSnapshot", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "approvalDecide", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "approvalRead", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "approveRegistration", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "cancel", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "configure", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "confirm", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "consentPolicy", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "create", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "customerReceipt", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "documents", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "drawerObservation", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "get", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "getConnectionStatus", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "identityChecked", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "list", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "manage", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "member", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "notifications", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "ops", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "opsProbe", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "printerOnline", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "process", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "publishReceipt", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "queue", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "quote", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "rateDraft", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "ratePolicySave", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "rates", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "registrations", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "reserveCustomer", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "retry", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "reversalRequest", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "save", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "saveTransaction", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "setDrawer", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "setPrinter", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "settings", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "status", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::StaffFile, "zzNoSuchAction", Some(Need::Permission(Permission::IdentityDocumentRead))),
    (EntryPath::Devices, "accept", None),
    (EntryPath::Devices, "adminAbandonRequest", None),
    (EntryPath::Devices, "adminCloseShift", None),
    (EntryPath::Devices, "adminCreateBranch", None),
    (EntryPath::Devices, "adminDeskConfig", None),
    (EntryPath::Devices, "adminMoveStock", None),
    (EntryPath::Devices, "adminOpenShift", None),
    (EntryPath::Devices, "adminSaveRate", None),
    (EntryPath::Devices, "adminSnapshot", None),
    (EntryPath::Devices, "approvalDecide", None),
    (EntryPath::Devices, "approvalRead", None),
    (EntryPath::Devices, "approveRegistration", None),
    (EntryPath::Devices, "cancel", None),
    (EntryPath::Devices, "configure", None),
    (EntryPath::Devices, "confirm", None),
    (EntryPath::Devices, "consentPolicy", None),
    (EntryPath::Devices, "create", None),
    (EntryPath::Devices, "customerReceipt", None),
    (EntryPath::Devices, "documents", None),
    (EntryPath::Devices, "drawerObservation", Some(Need::Permission(Permission::DeviceProcess))),
    (EntryPath::Devices, "get", None),
    (EntryPath::Devices, "getConnectionStatus", None),
    (EntryPath::Devices, "identityChecked", None),
    (EntryPath::Devices, "list", None),
    (EntryPath::Devices, "manage", None),
    (EntryPath::Devices, "member", None),
    (EntryPath::Devices, "notifications", None),
    (EntryPath::Devices, "ops", None),
    (EntryPath::Devices, "opsProbe", None),
    (EntryPath::Devices, "printerOnline", Some(Need::Permission(Permission::DevicePrinterOnline))),
    (EntryPath::Devices, "process", Some(Need::Permission(Permission::DeviceProcess))),
    (EntryPath::Devices, "publishReceipt", None),
    (EntryPath::Devices, "queue", Some(Need::Permission(Permission::DeviceQueue))),
    (EntryPath::Devices, "quote", None),
    (EntryPath::Devices, "rateDraft", None),
    (EntryPath::Devices, "ratePolicySave", None),
    (EntryPath::Devices, "rates", None),
    (EntryPath::Devices, "registrations", None),
    (EntryPath::Devices, "reserveCustomer", None),
    (EntryPath::Devices, "retry", Some(Need::Permission(Permission::DeviceProcess))),
    (EntryPath::Devices, "reversalRequest", None),
    (EntryPath::Devices, "save", None),
    (EntryPath::Devices, "saveTransaction", None),
    (EntryPath::Devices, "setDrawer", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::Devices, "setPrinter", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::Devices, "settings", None),
    (EntryPath::Devices, "status", None),
    (EntryPath::Devices, "zzNoSuchAction", None),
    (EntryPath::Kiosk, "accept", Some(Need::Permission(Permission::QueueOperate))),
    (EntryPath::Kiosk, "adminAbandonRequest", None),
    (EntryPath::Kiosk, "adminCloseShift", None),
    (EntryPath::Kiosk, "adminCreateBranch", None),
    (EntryPath::Kiosk, "adminDeskConfig", None),
    (EntryPath::Kiosk, "adminMoveStock", None),
    (EntryPath::Kiosk, "adminOpenShift", None),
    (EntryPath::Kiosk, "adminSaveRate", None),
    (EntryPath::Kiosk, "adminSnapshot", None),
    (EntryPath::Kiosk, "approvalDecide", None),
    (EntryPath::Kiosk, "approvalRead", None),
    (EntryPath::Kiosk, "approveRegistration", None),
    (
        EntryPath::Kiosk,
        "cancel",
        Some(Need::Permission(Permission::QueueOperate)),
    ), // KC-S #4176, kiosk-cancel.md §3.4
    (EntryPath::Kiosk, "configure", Some(Need::Permission(Permission::RatesDraft))),
    (EntryPath::Kiosk, "confirm", Some(Need::Permission(Permission::QueueOperate))),
    (EntryPath::Kiosk, "consentPolicy", None),
    (EntryPath::Kiosk, "create", None),
    (EntryPath::Kiosk, "customerCancel", Some(Need::Public)), // KC-S #4176, kiosk-cancel.md §3.4
    (EntryPath::Kiosk, "customerReceipt", Some(Need::Public)),
    (EntryPath::Kiosk, "documents", None),
    (EntryPath::Kiosk, "drawerObservation", None),
    (EntryPath::Kiosk, "get", Some(Need::Permission(Permission::QueueOperate))),
    (EntryPath::Kiosk, "getConnectionStatus", None),
    (EntryPath::Kiosk, "identityChecked", Some(Need::Permission(Permission::QueueOperate))),
    (EntryPath::Kiosk, "list", Some(Need::Permission(Permission::SignedIn))),
    (EntryPath::Kiosk, "manage", Some(Need::Permission(Permission::RatesDraft))),
    (EntryPath::Kiosk, "member", None),
    (EntryPath::Kiosk, "notifications", Some(Need::Permission(Permission::SignedIn))),
    (EntryPath::Kiosk, "ops", None),
    (EntryPath::Kiosk, "opsProbe", None),
    (EntryPath::Kiosk, "printerOnline", None),
    (EntryPath::Kiosk, "process", None),
    (EntryPath::Kiosk, "publishReceipt", Some(Need::Permission(Permission::QueueOperate))),
    (EntryPath::Kiosk, "queue", None),
    (EntryPath::Kiosk, "quote", Some(Need::Public)),
    (EntryPath::Kiosk, "rateDraft", None),
    (EntryPath::Kiosk, "ratePolicySave", None),
    (EntryPath::Kiosk, "rates", Some(Need::Public)),
    (EntryPath::Kiosk, "registrations", None),
    (EntryPath::Kiosk, "reserveCustomer", None),
    (EntryPath::Kiosk, "retry", None),
    (EntryPath::Kiosk, "reversalRequest", None),
    (EntryPath::Kiosk, "save", None),
    (EntryPath::Kiosk, "saveTransaction", None),
    (EntryPath::Kiosk, "setDrawer", None),
    (EntryPath::Kiosk, "setPrinter", None),
    (EntryPath::Kiosk, "settings", Some(Need::Public)), // KWQ-E3: kiosk-shared-phone.md D14, the screen reads its own settings
    (EntryPath::Kiosk, "status", Some(Need::Public)),
    (EntryPath::Kiosk, "zzNoSuchAction", None),
    (EntryPath::OwnerRoute, "accept", None),
    (EntryPath::OwnerRoute, "adminAbandonRequest", None),
    (EntryPath::OwnerRoute, "adminCloseShift", None),
    (EntryPath::OwnerRoute, "adminCreateBranch", None),
    (EntryPath::OwnerRoute, "adminDeskConfig", None),
    (EntryPath::OwnerRoute, "adminMoveStock", None),
    (EntryPath::OwnerRoute, "adminOpenShift", None),
    (EntryPath::OwnerRoute, "adminSaveRate", None),
    (EntryPath::OwnerRoute, "adminSnapshot", None),
    (EntryPath::OwnerRoute, "approvalDecide", None),
    (EntryPath::OwnerRoute, "approvalRead", None),
    (EntryPath::OwnerRoute, "approveRegistration", None),
    (EntryPath::OwnerRoute, "cancel", None),
    (EntryPath::OwnerRoute, "configure", None),
    (EntryPath::OwnerRoute, "confirm", None),
    (EntryPath::OwnerRoute, "consentPolicy", None),
    (EntryPath::OwnerRoute, "create", None),
    (EntryPath::OwnerRoute, "customerReceipt", None),
    (EntryPath::OwnerRoute, "documents", None),
    (EntryPath::OwnerRoute, "drawerObservation", None),
    (EntryPath::OwnerRoute, "get", None),
    (EntryPath::OwnerRoute, "getConnectionStatus", None),
    (EntryPath::OwnerRoute, "identityChecked", None),
    (EntryPath::OwnerRoute, "list", None),
    (EntryPath::OwnerRoute, "manage", None),
    (EntryPath::OwnerRoute, "member", None),
    (EntryPath::OwnerRoute, "notifications", None),
    (EntryPath::OwnerRoute, "ops", None),
    (EntryPath::OwnerRoute, "opsProbe", None),
    (EntryPath::OwnerRoute, "printerOnline", None),
    (EntryPath::OwnerRoute, "process", None),
    (EntryPath::OwnerRoute, "publishReceipt", None),
    (EntryPath::OwnerRoute, "queue", None),
    (EntryPath::OwnerRoute, "quote", None),
    (EntryPath::OwnerRoute, "rateDraft", None),
    (EntryPath::OwnerRoute, "ratePolicySave", None),
    (EntryPath::OwnerRoute, "rates", None),
    (EntryPath::OwnerRoute, "registrations", None),
    (EntryPath::OwnerRoute, "reserveCustomer", None),
    (EntryPath::OwnerRoute, "retry", None),
    (EntryPath::OwnerRoute, "reversalRequest", None),
    (EntryPath::OwnerRoute, "save", None),
    (EntryPath::OwnerRoute, "saveTransaction", None),
    (EntryPath::OwnerRoute, "setDrawer", None),
    (EntryPath::OwnerRoute, "setPrinter", None),
    (EntryPath::OwnerRoute, "settings", None),
    (EntryPath::OwnerRoute, "status", None),
    (EntryPath::OwnerRoute, "zzNoSuchAction", None),
    (EntryPath::UserRoute, "accept", None),
    (EntryPath::UserRoute, "adminAbandonRequest", None),
    (EntryPath::UserRoute, "adminCloseShift", None),
    (EntryPath::UserRoute, "adminCreateBranch", None),
    (EntryPath::UserRoute, "adminDeskConfig", None),
    (EntryPath::UserRoute, "adminMoveStock", None),
    (EntryPath::UserRoute, "adminOpenShift", None),
    (EntryPath::UserRoute, "adminSaveRate", None),
    (EntryPath::UserRoute, "adminSnapshot", None),
    (EntryPath::UserRoute, "approvalDecide", None),
    (EntryPath::UserRoute, "approvalRead", None),
    (EntryPath::UserRoute, "approveRegistration", None),
    (EntryPath::UserRoute, "cancel", None),
    (EntryPath::UserRoute, "configure", None),
    (EntryPath::UserRoute, "confirm", None),
    (EntryPath::UserRoute, "consentPolicy", None),
    (EntryPath::UserRoute, "create", None),
    (EntryPath::UserRoute, "customerReceipt", None),
    (EntryPath::UserRoute, "documents", Some(Need::Permission(Permission::DocumentsList))),
    (EntryPath::UserRoute, "drawerObservation", None),
    (EntryPath::UserRoute, "get", None),
    (EntryPath::UserRoute, "getConnectionStatus", None),
    (EntryPath::UserRoute, "identityChecked", None),
    (EntryPath::UserRoute, "list", None),
    (EntryPath::UserRoute, "manage", None),
    (EntryPath::UserRoute, "member", None),
    (EntryPath::UserRoute, "notifications", None),
    (EntryPath::UserRoute, "ops", None),
    (EntryPath::UserRoute, "opsProbe", None),
    (EntryPath::UserRoute, "printerOnline", None),
    (EntryPath::UserRoute, "process", None),
    (EntryPath::UserRoute, "publishReceipt", None),
    (EntryPath::UserRoute, "queue", None),
    (EntryPath::UserRoute, "quote", None),
    (EntryPath::UserRoute, "rateDraft", None),
    (EntryPath::UserRoute, "ratePolicySave", None),
    (EntryPath::UserRoute, "rates", None),
    (EntryPath::UserRoute, "registrations", None),
    (EntryPath::UserRoute, "reserveCustomer", None),
    (EntryPath::UserRoute, "retry", None),
    (EntryPath::UserRoute, "reversalRequest", None),
    (EntryPath::UserRoute, "save", None),
    (EntryPath::UserRoute, "saveTransaction", None),
    (EntryPath::UserRoute, "setDrawer", None),
    (EntryPath::UserRoute, "setPrinter", None),
    (EntryPath::UserRoute, "settings", None),
    (EntryPath::UserRoute, "status", None),
    (EntryPath::UserRoute, "zzNoSuchAction", None),
    (EntryPath::PaymentMethods, "accept", None),
    (EntryPath::PaymentMethods, "adminAbandonRequest", None),
    (EntryPath::PaymentMethods, "adminCloseShift", None),
    (EntryPath::PaymentMethods, "adminCreateBranch", None),
    (EntryPath::PaymentMethods, "adminDeskConfig", None),
    (EntryPath::PaymentMethods, "adminMoveStock", None),
    (EntryPath::PaymentMethods, "adminOpenShift", None),
    (EntryPath::PaymentMethods, "adminSaveRate", None),
    (EntryPath::PaymentMethods, "adminSnapshot", None),
    (EntryPath::PaymentMethods, "approvalDecide", None),
    (EntryPath::PaymentMethods, "approvalRead", None),
    (EntryPath::PaymentMethods, "approveRegistration", None),
    (EntryPath::PaymentMethods, "cancel", None),
    (EntryPath::PaymentMethods, "configure", None),
    (EntryPath::PaymentMethods, "confirm", None),
    (EntryPath::PaymentMethods, "consentPolicy", None),
    (EntryPath::PaymentMethods, "create", None),
    (EntryPath::PaymentMethods, "customerReceipt", None),
    (EntryPath::PaymentMethods, "documents", None),
    (EntryPath::PaymentMethods, "drawerObservation", None),
    (EntryPath::PaymentMethods, "get", None),
    (EntryPath::PaymentMethods, "getConnectionStatus", None),
    (EntryPath::PaymentMethods, "identityChecked", None),
    (EntryPath::PaymentMethods, "list", Some(Need::Permission(Permission::SignedIn))),
    (EntryPath::PaymentMethods, "manage", None),
    (EntryPath::PaymentMethods, "member", None),
    (EntryPath::PaymentMethods, "notifications", None),
    (EntryPath::PaymentMethods, "ops", None),
    (EntryPath::PaymentMethods, "opsProbe", None),
    (EntryPath::PaymentMethods, "printerOnline", None),
    (EntryPath::PaymentMethods, "process", None),
    (EntryPath::PaymentMethods, "publishReceipt", None),
    (EntryPath::PaymentMethods, "queue", None),
    (EntryPath::PaymentMethods, "quote", None),
    (EntryPath::PaymentMethods, "rateDraft", None),
    (EntryPath::PaymentMethods, "ratePolicySave", None),
    (EntryPath::PaymentMethods, "rates", None),
    (EntryPath::PaymentMethods, "registrations", None),
    (EntryPath::PaymentMethods, "reserveCustomer", None),
    (EntryPath::PaymentMethods, "retry", None),
    (EntryPath::PaymentMethods, "reversalRequest", None),
    (EntryPath::PaymentMethods, "save", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::PaymentMethods, "saveTransaction", None),
    (EntryPath::PaymentMethods, "setDrawer", None),
    (EntryPath::PaymentMethods, "setPrinter", None),
    (EntryPath::PaymentMethods, "settings", None),
    (EntryPath::PaymentMethods, "status", None),
    (EntryPath::PaymentMethods, "zzNoSuchAction", None),
    (EntryPath::DrawerPassport, "accept", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminAbandonRequest", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminCloseShift", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminCreateBranch", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminDeskConfig", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminMoveStock", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminOpenShift", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminSaveRate", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "adminSnapshot", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "approvalDecide", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "approvalRead", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "approveRegistration", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "cancel", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "configure", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "confirm", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "consentPolicy", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "create", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "customerReceipt", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "documents", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "drawerObservation", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "get", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "getConnectionStatus", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "identityChecked", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "list", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "manage", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "member", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "notifications", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "ops", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "opsProbe", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "printerOnline", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "process", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "publishReceipt", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "queue", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "quote", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "rateDraft", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "ratePolicySave", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "rates", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "registrations", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "reserveCustomer", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "retry", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "reversalRequest", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "save", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "saveTransaction", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "setDrawer", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "setPrinter", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "settings", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "status", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DrawerPassport, "zzNoSuchAction", Some(Need::Permission(Permission::DrawerPassport))),
    (EntryPath::DeviceSetup, "accept", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminAbandonRequest", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminCloseShift", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminCreateBranch", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminDeskConfig", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminMoveStock", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminOpenShift", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminSaveRate", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "adminSnapshot", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "approvalDecide", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "approvalRead", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "approveRegistration", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "cancel", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "configure", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "confirm", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "consentPolicy", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "create", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "customerReceipt", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "documents", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "drawerObservation", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "get", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "getConnectionStatus", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "identityChecked", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "list", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "manage", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "member", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "notifications", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "ops", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "opsProbe", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "printerOnline", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "process", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "publishReceipt", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "queue", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "quote", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "rateDraft", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "ratePolicySave", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "rates", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "registrations", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "reserveCustomer", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "retry", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "reversalRequest", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "save", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "saveTransaction", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "setDrawer", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "setPrinter", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "settings", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "status", Some(Need::Permission(Permission::SystemSettings))),
    (EntryPath::DeviceSetup, "zzNoSuchAction", Some(Need::Permission(Permission::SystemSettings))),
];
