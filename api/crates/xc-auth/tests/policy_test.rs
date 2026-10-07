//! Who may do what: the authorisation rule, with no database and no HTTP.
//!
//! Provided with ticket T4a. These tests are the specification: make them pass,
//! do not edit, delete or weaken them. `fixtures/v2_policy_cases.json` is what v2
//! answered when each caller asked for each action on each of its entry paths
//! (`api/tools/v2-fixtures/gen_policy.mjs` drives v2's own request handler); the
//! tests below it are the rules a table of 1,370 rows does not show at a glance.
//!
//! v2 decides in more than one place and the places do not agree. Nothing here
//! flattens them: a question is always asked *on a path*.
//!
//! Edited by the author of ticket POL (design `docs/design/xchange-v3.md` §5.5.7): the fixture
//! is re-recorded from the 10-06 bundle (`XC-Desk-v2-Passport-Drawer-20261006.zip`, sha256
//! `6514b4c8…4375f570`) by the same script, which now also asks the two routes that bundle adds
//! (`/api/payment-methods`, `/api/drawer-passport`). Against the 10-05 recording the only
//! answers that changed are six: `list` on the kiosk is let through for RATES, MONITOR, FINANCE,
//! SYSTEM, DOCUMENTS, DOCUMENT_REVIEW (`server.mjs:8026` of the 10-06 bundle). The counts below,
//! the number of sentences and `the_kiosk_queue_is_for_front_backend_manager_and_the_owner`
//! follow that recording; nothing else here changed.

mod policy_common;

use std::collections::BTreeSet;

use policy_common::*;

const B2: &str = "B2";
const B9: &str = "B9";

fn front() -> Actor {
    staff(Role::Front, B2)
}

/// Every caller the fixture knows: nobody, the owner, one employee per role.
fn everyone() -> Vec<Option<Actor>> {
    let mut all = vec![None, Some(owner())];
    all.extend(Role::STAFF.into_iter().map(|r| Some(staff(r, B2))));
    all
}

fn allowed_roles(on: EntryPath, name: &str) -> Vec<&'static str> {
    Role::STAFF
        .into_iter()
        .filter(|r| ask(Some(&staff(*r, B2)), on, name).is_ok())
        .map(Role::as_str)
        .collect()
}

// ── the fixture ──────────────────────────────────────────────────────────────

#[test]
fn the_fixture_is_whole() {
    let f = fixture();
    assert_eq!(f.cases.len(), 1425);
    assert_eq!(f.cases.len(), f.counts.cases);
    assert_eq!(f.cases.iter().filter(|c| c.allowed).count(), 464);
    assert_eq!(f.counts.allowed, 464);
    assert_eq!(f.counts.refused, 961);
    assert_eq!(
        f.cases.iter().filter(|c| c.redirect.is_some()).count(),
        f.counts.redirects
    );
    assert_eq!(f.counts.redirects, 15);
    assert_eq!(f.branch_cases.len(), 285);
    assert_eq!(f.branch_cases.len(), f.counts.branch_cases);
    assert_eq!(f.refusals.len(), 22);
    assert_eq!(f.refusals.len(), f.counts.distinct_refusals);
    assert_eq!((f.counts.paths, f.counts.actors), (14, 11));
    assert_eq!(f.counts.disagreements, 137);

    // every path, and on every path somebody allowed and somebody refused
    for p in PATHS {
        let rows: Vec<_> = f.cases.iter().filter(|c| c.path == path_name(p)).collect();
        assert!(
            rows.iter().any(|c| c.allowed),
            "{} never allows",
            path_name(p)
        );
        assert!(
            rows.iter().any(|c| !c.allowed),
            "{} never refuses",
            path_name(p)
        );
    }
    let named: BTreeSet<&str> = f.cases.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(named.len(), PATHS.len());

    // every caller, each of them allowed somewhere and refused somewhere
    let callers: BTreeSet<&str> = f.cases.iter().map(|c| c.actor.as_str()).collect();
    assert_eq!(callers.len(), 11);
    for who in callers {
        let rows: Vec<_> = f.cases.iter().filter(|c| c.actor == who).collect();
        assert!(rows.iter().any(|c| c.allowed), "{who} is never allowed");
        assert!(rows.iter().any(|c| !c.allowed), "{who} is never refused");
    }
}

#[test]
fn every_answer_v2_gave_is_given_again() {
    let f = fixture();
    let mut wrong = Vec::new();
    for c in &f.cases {
        let who = caller(&c.actor, c.own.as_deref());
        let got = answer(ask(who.as_ref(), path(&c.path), &c.action));
        let want = recorded(c);
        if got != want {
            wrong.push(format!(
                "{} {:?} as {} ({}): v2 {:?}, here {:?}",
                c.path, c.action, c.actor, c.route, want, got
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} answers differ from v2; the first ones:\n{}",
        wrong.len(),
        f.cases.len(),
        wrong
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_sentences_said_are_exactly_v2s_twenty_two() {
    let f = fixture();
    let want: BTreeSet<(String, u16)> = f
        .refusals
        .iter()
        .map(|r| (r.error.clone(), r.status))
        .collect();
    let mut got = BTreeSet::new();
    for c in &f.cases {
        let who = caller(&c.actor, c.own.as_deref());
        if let Err(e) = ask(who.as_ref(), path(&c.path), &c.action) {
            got.insert((e.to_string(), e.status()));
        }
    }
    assert_eq!(got, want);
    assert_eq!(want.len(), 22);
}

#[test]
fn every_call_runs_on_the_branch_v2_ran_it_on() {
    let f = fixture();
    let mut seen = 0;
    for c in f.cases.iter().chain(&f.branch_cases) {
        let Some(want) = &c.branch else { continue };
        let who = caller(&c.actor, c.own.as_deref());
        let named = c.named.as_deref().map(branch);
        assert_eq!(
            Policy::branch(who.as_ref(), named.as_ref()),
            branch(want),
            "{} {} as {} naming {:?}",
            c.route,
            c.action,
            c.actor,
            c.named
        );
        seen += 1;
    }
    assert_eq!(seen, 689, "rows of the fixture that show a branch");
}

#[test]
fn which_branch_an_employee_belongs_to_changes_no_answer() {
    let f = fixture();
    for c in f.cases.iter().filter(|c| c.own.is_some()) {
        for home in ["MAIN", B9, "B1700000000000-x"] {
            let who = caller(&c.actor, Some(home));
            assert_eq!(
                answer(ask(who.as_ref(), path(&c.path), &c.action)),
                recorded(c),
                "{} {:?} as {} of {home}",
                c.path,
                c.action,
                c.actor
            );
        }
    }
}

// ── roles ────────────────────────────────────────────────────────────────────

#[test]
fn roles_are_spelled_as_v2_spells_them() {
    let spelled: Vec<&str> = Role::STAFF.into_iter().map(Role::as_str).collect();
    assert_eq!(
        spelled,
        [
            "FRONT",
            "BACKEND",
            "MANAGER",
            "RATES",
            "MONITOR",
            "FINANCE",
            "SYSTEM",
            "DOCUMENTS",
            "DOCUMENT_REVIEW"
        ]
    );
    assert_eq!(spelled, fixture().roles);
    assert_eq!(Role::Owner.as_str(), "OWNER");
    assert!(!Role::STAFF.contains(&Role::Owner));
    for r in Role::STAFF.into_iter().chain([Role::Owner]) {
        assert_eq!(Role::parse(r.as_str()), Some(r));
    }
}

#[test]
fn a_role_v2_does_not_have_is_no_role() {
    for text in [
        "",
        "INTERN",
        "front",
        "Front",
        " FRONT",
        "FRONT ",
        "DOCUMENT-REVIEW",
        "DOCUMENTREVIEW",
        "owner",
        "ADMIN",
        "FRONT,BACKEND",
    ] {
        assert_eq!(Role::parse(text), None, "{text:?}");
    }
}

// ── who is calling ───────────────────────────────────────────────────────────

#[test]
fn the_owner_has_no_branch_and_an_employee_always_has_one() {
    let o = owner();
    assert_eq!(
        (o.id(), o.kind(), o.role(), o.branch_id()),
        ("owner", ActorKind::Owner, Role::Owner, None)
    );
    let e = staff(Role::Monitor, B2);
    assert_eq!(
        (e.id(), e.kind(), e.role(), e.branch_id()),
        (
            "E-MONITOR",
            ActorKind::Staff,
            Role::Monitor,
            Some(&branch(B2))
        )
    );
}

#[test]
fn nobody_is_an_employee_with_the_owners_role() {
    assert_eq!(Actor::staff("E-1", Role::Owner, branch(B2)), None);
    for r in Role::STAFF {
        assert!(Actor::staff("E-1", r, branch(B2)).is_some());
    }
}

// ── the branch ───────────────────────────────────────────────────────────────

#[test]
fn an_employee_is_pinned_to_their_own_branch_whatever_they_name() {
    for r in Role::STAFF {
        let e = staff(r, B2);
        assert_eq!(Policy::branch(Some(&e), Some(&branch(B9))), branch(B2));
        assert_eq!(Policy::branch(Some(&e), Some(&branch("MAIN"))), branch(B2));
        assert_eq!(Policy::branch(Some(&e), Some(&branch(B2))), branch(B2));
        assert_eq!(Policy::branch(Some(&e), None), branch(B2));
    }
    // an employee of MAIN is pinned there too: MAIN is a branch, not a wildcard
    let main = staff(Role::Manager, "MAIN");
    assert_eq!(
        Policy::branch(Some(&main), Some(&branch(B9))),
        branch("MAIN")
    );
}

#[test]
fn naming_another_branch_is_not_refused_it_is_ignored() {
    // v2 never answers "wrong branch" to an employee: it overwrites the branch and goes on.
    let e = front();
    assert_eq!(ask(Some(&e), EntryPath::Rpc, "saveTransaction"), Ok(()));
    assert_eq!(
        ask(Some(&e), EntryPath::StaffRpc, "saveTransaction"),
        Ok(())
    );
    assert_eq!(Policy::branch(Some(&e), Some(&branch(B9))), branch(B2));
}

#[test]
fn the_owner_works_on_the_branch_named_and_on_main_when_none_is() {
    let o = owner();
    assert_eq!(Policy::branch(Some(&o), Some(&branch(B9))), branch(B9));
    assert_eq!(Policy::branch(Some(&o), Some(&branch(B2))), branch(B2));
    assert_eq!(Policy::branch(Some(&o), None), branch("MAIN"));
}

#[test]
fn a_caller_who_is_not_signed_in_is_on_the_branch_named() {
    // the customer kiosk: nobody is signed in and the screen says which branch it stands in
    assert_eq!(Policy::branch(None, Some(&branch(B9))), branch(B9));
    assert_eq!(Policy::branch(None, None), branch("MAIN"));
}

// ── the two RPC paths keep their own lists ───────────────────────────────────

#[test]
fn opening_and_closing_a_shift_depends_on_the_path() {
    for name in ["adminOpenShift", "adminCloseShift"] {
        assert_eq!(allowed_roles(EntryPath::Rpc, name), ["BACKEND", "MANAGER"]);
        assert!(allowed_roles(EntryPath::StaffRpc, name).is_empty());
    }
    assert_eq!(allowed_roles(EntryPath::Rpc, "adminMoveStock"), ["FINANCE"]);
    assert!(allowed_roles(EntryPath::StaffRpc, "adminMoveStock").is_empty());
}

#[test]
fn the_desk_configuration_depends_on_the_path() {
    assert_eq!(allowed_roles(EntryPath::Rpc, "adminDeskConfig").len(), 9);
    assert_eq!(
        allowed_roles(EntryPath::StaffRpc, "adminDeskConfig"),
        ["FRONT"]
    );
    assert_eq!(
        said(ask(
            Some(&staff(Role::Monitor, B2)),
            EntryPath::StaffRpc,
            "adminDeskConfig"
        )),
        (FRONT_ONLY.to_owned(), 400)
    );
}

#[test]
fn reading_a_transactions_documents_depends_on_the_path() {
    // not on /api/rpc at all for an employee, nor for the owner
    assert!(allowed_roles(EntryPath::Rpc, "documents").is_empty());
    assert_eq!(
        said(ask(Some(&owner()), EntryPath::Rpc, "documents")),
        (INVALID_REQUEST.to_owned(), 400)
    );
    // on the staff bridge for everybody but Rates and System
    assert_eq!(
        allowed_roles(EntryPath::StaffRpc, "documents"),
        [
            "FRONT",
            "BACKEND",
            "MANAGER",
            "MONITOR",
            "FINANCE",
            "DOCUMENTS",
            "DOCUMENT_REVIEW"
        ]
    );
    for r in [Role::Rates, Role::System] {
        assert_eq!(
            said(ask(Some(&staff(r, B2)), EntryPath::StaffRpc, "documents")),
            (DOCUMENTS_NOT_READABLE.to_owned(), 400)
        );
        // and the documents page lets the same two in
        assert_eq!(
            ask(Some(&staff(r, B2)), EntryPath::UserRoute, "documents"),
            Ok(())
        );
    }
}

#[test]
fn the_two_paths_refuse_in_sentences_one_word_apart() {
    let m = staff(Role::Monitor, B2);
    let on_rpc = said(ask(Some(&m), EntryPath::Rpc, "saveTransaction"));
    let on_bridge = said(ask(Some(&m), EntryPath::StaffRpc, "saveTransaction"));
    assert_eq!(on_rpc, ("บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้".to_owned(), 400));
    assert_eq!(on_bridge, ("บทบาทนี้ไม่มีสิทธิ์ทำรายการ".to_owned(), 400));
    assert_eq!(on_rpc.0, format!("{}นี้", on_bridge.0));
    assert_eq!((on_rpc.0.len(), on_bridge.0.len()), (90, 81)); // bytes of UTF-8
}

#[test]
fn what_each_role_may_write_on_both_paths() {
    for on in [EntryPath::Rpc, EntryPath::StaffRpc] {
        assert_eq!(allowed_roles(on, "reserveCustomer"), ["FRONT"]);
        assert_eq!(allowed_roles(on, "saveTransaction"), ["FRONT"]);
        assert_eq!(allowed_roles(on, "adminSaveRate"), ["RATES"]);
        assert!(allowed_roles(on, "adminCreateBranch").is_empty());
        assert!(allowed_roles(on, "adminAbandonRequest").is_empty());
        assert_eq!(allowed_roles(on, "adminSnapshot").len(), 9);
        assert_eq!(allowed_roles(on, "getConnectionStatus").len(), 9);
    }
}

// ── the owner ────────────────────────────────────────────────────────────────

#[test]
fn the_owner_may_do_all_eleven_actions_of_the_owner_rpc() {
    let o = owner();
    for name in [
        "adminSnapshot",
        "adminDeskConfig",
        "adminOpenShift",
        "adminCloseShift",
        "adminMoveStock",
        "adminSaveRate",
        "adminCreateBranch",
        "adminAbandonRequest",
        "getConnectionStatus",
        "reserveCustomer",
        "saveTransaction",
    ] {
        assert_eq!(ask(Some(&o), EntryPath::Rpc, name), Ok(()), "{name}");
    }
}

#[test]
fn the_owner_is_let_through_wherever_a_signed_in_caller_is() {
    let o = owner();
    for (on, name) in [
        (EntryPath::Upload, "upload"),
        (EntryPath::File, "file"),
        (EntryPath::Devices, "printerOnline"),
        (EntryPath::Devices, "queue"),
        (EntryPath::Kiosk, "configure"),
        (EntryPath::Kiosk, "accept"),
        (EntryPath::OwnerRoute, "create"),
        (EntryPath::OwnerRoute, "reference-rates"),
        (EntryPath::UserRoute, "documents"),
        (EntryPath::Custody, "opsSetEntity"),
        (EntryPath::Custody, "opsEnable"),
    ] {
        assert_eq!(ask(Some(&o), on, name), Ok(()), "{} {name}", path_name(on));
    }
}

#[test]
fn the_owner_is_not_an_employee() {
    // the staff routes know employees only; the owner's session means nothing there
    let o = owner();
    for (on, name) in [
        (EntryPath::StaffRpc, "adminSnapshot"),
        (EntryPath::StaffRpc, "opsClock"),
        (EntryPath::StaffUpload, "upload"),
        (EntryPath::StaffFile, "file"),
        (EntryPath::StaffDesk, "desk"),
    ] {
        assert_eq!(
            said(ask(Some(&o), on, name)),
            (EMPLOYEE_AUTH.to_owned(), 401),
            "{} {name}",
            path_name(on)
        );
    }
    // and the owner RPC has no custody action at all
    assert_eq!(
        said(ask(Some(&o), EntryPath::Rpc, "opsEnable")),
        (INVALID_REQUEST.to_owned(), 400)
    );
}

#[test]
fn only_the_owner_passes_an_owner_route() {
    for name in ["list", "create", "reset", "disable", "entity", "enable"] {
        assert_eq!(ask(Some(&owner()), EntryPath::OwnerRoute, name), Ok(()));
        for r in Role::STAFF {
            assert_eq!(
                said(ask(Some(&staff(r, B2)), EntryPath::OwnerRoute, name)),
                (OWNER_ONLY.to_owned(), 403)
            );
        }
        assert_eq!(
            said(ask(None, EntryPath::OwnerRoute, name)),
            (SIGN_IN.to_owned(), 401)
        );
    }
}

// ── names v2 does not know ───────────────────────────────────────────────────

#[test]
fn an_unknown_action_is_refused_in_the_words_of_its_path() {
    let m = staff(Role::Manager, B2);
    for name in ["nope", "", "adminsnapshot", "adminSnapshot ", "Ops"] {
        assert_eq!(
            said(ask(Some(&owner()), EntryPath::Rpc, name)),
            (INVALID_REQUEST.to_owned(), 400),
            "{name:?}"
        );
        assert_eq!(
            said(ask(Some(&m), EntryPath::Rpc, name)),
            (ROLE_NOT_ALLOWED_HERE.to_owned(), 400),
            "{name:?}"
        );
        assert_eq!(
            said(ask(Some(&m), EntryPath::StaffRpc, name)),
            (ROLE_NOT_ALLOWED.to_owned(), 400),
            "{name:?}"
        );
    }
    assert_eq!(
        said(ask(Some(&m), EntryPath::Kiosk, "nope")),
        (KIOSK_UNKNOWN.to_owned(), 400)
    );
    assert_eq!(
        said(ask(
            Some(&staff(Role::Finance, B2)),
            EntryPath::Kiosk,
            "nope"
        )),
        (KIOSK_STAFF.to_owned(), 400)
    );
    assert_eq!(
        said(ask(None, EntryPath::Kiosk, "nope")),
        (KIOSK_STAFF.to_owned(), 400)
    );
    assert_eq!(
        said(ask(Some(&m), EntryPath::Custody, "opsNope")),
        (INVALID_REQUEST.to_owned(), 400)
    );
}

#[test]
fn the_staff_bridge_lets_through_whatever_starts_with_ops() {
    // the custody module, not the bridge, decides those — including names it does not know
    for r in Role::STAFF {
        let e = staff(r, B2);
        for name in ["opsClock", "opsCommit", "opsNope", "ops", "opsenable"] {
            assert_eq!(ask(Some(&e), EntryPath::StaffRpc, name), Ok(()), "{name}");
        }
        // except the two the owner alone starts, which the bridge refuses in its own words
        for name in ["opsEnable", "opsSetEntity"] {
            assert_eq!(
                said(ask(Some(&e), EntryPath::StaffRpc, name)),
                (OWNER_ENABLES.to_owned(), 400)
            );
        }
        // and /api/rpc lets no employee near any of them
        for name in ["opsClock", "ops", "opsEnable"] {
            assert_eq!(
                said(ask(Some(&e), EntryPath::Rpc, name)),
                (ROLE_NOT_ALLOWED_HERE.to_owned(), 400)
            );
        }
    }
}

#[test]
fn the_device_route_does_not_stop_a_name_it_has_no_list_for() {
    // v2 looks the name up in its list and lets a name without a list through; the device
    // service, further on, is what answers that it does not know the action
    for who in everyone().iter().flatten() {
        assert_eq!(ask(Some(who), EntryPath::Devices, "nope"), Ok(()));
        assert_eq!(ask(Some(who), EntryPath::Devices, "snapshot"), Ok(()));
    }
    assert_eq!(
        said(ask(None, EntryPath::Devices, "nope")),
        (SIGN_IN.to_owned(), 401)
    );
}

#[test]
fn paths_with_one_purpose_do_not_read_the_action() {
    let one_purpose = [
        (EntryPath::Upload, "upload"),
        (EntryPath::StaffUpload, "upload"),
        (EntryPath::File, "file"),
        (EntryPath::StaffFile, "file"),
        (EntryPath::StaffDesk, "desk"),
        (EntryPath::OwnerRoute, "list"),
        (EntryPath::UserRoute, "documents"),
    ];
    for (on, usual) in one_purpose {
        for who in everyone() {
            let want = answer(ask(who.as_ref(), on, usual));
            for name in ["", "nope", "adminSnapshot", "opsClock", "saveTransaction"] {
                assert_eq!(
                    answer(ask(who.as_ref(), on, name)),
                    want,
                    "{} {name:?}",
                    path_name(on)
                );
            }
        }
    }
}

// ── files, uploads, the desk page ────────────────────────────────────────────

#[test]
fn front_and_finance_upload_evidence_and_nobody_else_on_the_staff() {
    for on in [EntryPath::Upload, EntryPath::StaffUpload] {
        assert_eq!(allowed_roles(on, "upload"), ["FRONT", "FINANCE"]);
        assert_eq!(
            said(ask(Some(&staff(Role::Backend, B2)), on, "upload")),
            (UPLOAD_NOT_ALLOWED.to_owned(), 400)
        );
    }
    assert_eq!(
        said(ask(None, EntryPath::Upload, "upload")),
        (SIGN_IN.to_owned(), 401)
    );
    assert_eq!(
        said(ask(None, EntryPath::StaffUpload, "upload")),
        (EMPLOYEE_AUTH.to_owned(), 401)
    );
}

#[test]
fn rates_and_system_read_no_customer_file_and_each_path_says_so_differently() {
    for r in [Role::Rates, Role::System] {
        let e = staff(r, B2);
        assert_eq!(
            said(ask(Some(&e), EntryPath::File, "file")),
            (NOT_FOUND.to_owned(), 404)
        );
        assert_eq!(
            said(ask(Some(&e), EntryPath::StaffFile, "file")),
            (FILES_NOT_READABLE.to_owned(), 400)
        );
    }
    for on in [EntryPath::File, EntryPath::StaffFile] {
        assert_eq!(allowed_roles(on, "file").len(), 7);
    }
}

#[test]
fn the_staff_desk_page_is_fronts() {
    assert_eq!(allowed_roles(EntryPath::StaffDesk, "desk"), ["FRONT"]);
    assert_eq!(
        said(ask(
            Some(&staff(Role::Backend, B2)),
            EntryPath::StaffDesk,
            "desk"
        )),
        (FRONT_ONLY.to_owned(), 400)
    );
    assert_eq!(
        said(ask(None, EntryPath::StaffDesk, "desk")),
        (EMPLOYEE_AUTH.to_owned(), 401)
    );
}

// ── devices and the kiosk ────────────────────────────────────────────────────

#[test]
fn device_actions_have_their_own_role_lists() {
    assert_eq!(
        allowed_roles(EntryPath::Devices, "queue"),
        ["FRONT", "BACKEND", "MANAGER"]
    );
    for name in ["process", "retry", "drawerObservation"] {
        assert_eq!(
            allowed_roles(EntryPath::Devices, name),
            ["FRONT", "BACKEND"]
        );
    }
    assert_eq!(
        allowed_roles(EntryPath::Devices, "printerOnline"),
        ["BACKEND"]
    );
    assert_eq!(
        said(ask(Some(&front()), EntryPath::Devices, "printerOnline")),
        (DEVICE_STEP_NOT_ALLOWED.to_owned(), 400)
    );
}

#[test]
fn the_customer_kiosk_needs_nobody_signed_in() {
    for name in ["rates", "quote", "confirm", "status", "customerReceipt"] {
        for who in everyone() {
            assert_eq!(ask(who.as_ref(), EntryPath::Kiosk, name), Ok(()), "{name}");
        }
    }
}

#[test]
fn the_kiosk_queue_is_for_front_backend_manager_and_the_owner() {
    for name in ["accept", "get", "identityChecked", "publishReceipt"] {
        assert_eq!(
            allowed_roles(EntryPath::Kiosk, name),
            ["FRONT", "BACKEND", "MANAGER"]
        );
        assert_eq!(ask(Some(&owner()), EntryPath::Kiosk, name), Ok(()));
        assert_eq!(
            said(ask(None, EntryPath::Kiosk, name)),
            (KIOSK_STAFF.to_owned(), 400)
        );
    }
    // the waiting count, and since the 10-06 bundle the queue itself, to anybody signed in
    for name in ["notifications", "list"] {
        assert_eq!(allowed_roles(EntryPath::Kiosk, name).len(), 9, "{name}");
        assert_eq!(
            ask(Some(&owner()), EntryPath::Kiosk, name),
            Ok(()),
            "{name}"
        );
        assert_eq!(
            said(ask(None, EntryPath::Kiosk, name)),
            (KIOSK_STAFF.to_owned(), 400),
            "{name}"
        );
    }
    // which currencies the kiosk offers: the owner and Rates
    for name in ["manage", "configure"] {
        assert_eq!(allowed_roles(EntryPath::Kiosk, name), ["RATES"]);
        assert_eq!(
            said(ask(Some(&front()), EntryPath::Kiosk, name)),
            (KIOSK_OWNER_OR_RATES.to_owned(), 400)
        );
        assert_eq!(
            said(ask(None, EntryPath::Kiosk, name)),
            (KIOSK_OWNER_OR_RATES.to_owned(), 400)
        );
    }
}

// ── custody: what the role alone decides ─────────────────────────────────────

#[test]
fn each_custody_action_has_its_roles() {
    let table: [(&str, &[&str]); 12] = [
        ("opsBeginTransaction", &["FRONT"]),
        ("opsTarget", &["FINANCE"]),
        ("opsStartService", &["BACKEND"]),
        ("opsCutoff", &["BACKEND"]),
        ("opsPropose", &["FRONT", "FINANCE"]),
        ("opsHold", &["FRONT", "MONITOR"]),
        ("opsRelease", &["MONITOR"]),
        ("opsReject", &["BACKEND", "MONITOR"]),
        ("opsSign", &["FRONT", "MANAGER", "MONITOR", "FINANCE"]),
        ("opsDispatch", &["BACKEND"]),
        ("opsCommit", &["BACKEND"]),
        ("opsReconcile", &["FINANCE"]),
    ];
    for (name, roles) in table {
        assert_eq!(allowed_roles(EntryPath::Custody, name), roles, "{name}");
    }
    assert_eq!(allowed_roles(EntryPath::Custody, "opsClock").len(), 9);
    assert!(allowed_roles(EntryPath::Custody, "opsEnable").is_empty());
    assert!(allowed_roles(EntryPath::Custody, "opsSetEntity").is_empty());
}

#[test]
fn signing_is_refused_in_three_different_sentences() {
    let sign = |who: &Actor| said(ask(Some(who), EntryPath::Custody, "opsSign"));
    assert_eq!(
        sign(&staff(Role::Backend, B2)),
        (BACKEND_USES_COMMIT.to_owned(), 400)
    );
    for r in [
        Role::Rates,
        Role::System,
        Role::Documents,
        Role::DocumentReview,
    ] {
        assert_eq!(sign(&staff(r, B2)), (SIGN_NOT_ALLOWED.to_owned(), 400));
    }
    assert_eq!(sign(&owner()), (CLOCK_IN_FIRST.to_owned(), 400));
}

#[test]
fn the_owner_starts_custody_and_does_nothing_else_in_it() {
    let o = owner();
    assert_eq!(
        said(ask(Some(&o), EntryPath::Custody, "opsClock")),
        (EMPLOYEE_ACCOUNT.to_owned(), 400)
    );
    for name in ["opsTarget", "opsPropose", "opsCommit", "opsNope"] {
        assert_eq!(
            said(ask(Some(&o), EntryPath::Custody, name)),
            (CLOCK_IN_FIRST.to_owned(), 400),
            "{name}"
        );
    }
    assert_eq!(
        said(ask(Some(&front()), EntryPath::Custody, "opsEnable")),
        (ROLE_NOT_ALLOWED.to_owned(), 400)
    );
}

// ── the refusal ──────────────────────────────────────────────────────────────

#[test]
fn a_refusal_carries_the_status_v2_answered_with() {
    let m = staff(Role::Monitor, B2);
    let status = |got: Result<(), PolicyError>| said(got).1;
    assert_eq!(status(ask(None, EntryPath::Rpc, "adminSnapshot")), 401);
    assert_eq!(status(ask(None, EntryPath::UserRoute, "documents")), 401);
    assert_eq!(status(ask(None, EntryPath::File, "file")), 401);
    assert_eq!(status(ask(None, EntryPath::StaffRpc, "adminSnapshot")), 401);
    assert_eq!(status(ask(Some(&m), EntryPath::OwnerRoute, "list")), 403);
    assert_eq!(
        status(ask(Some(&staff(Role::System, B2)), EntryPath::File, "file")),
        404
    );
    assert_eq!(status(ask(Some(&m), EntryPath::Rpc, "adminSaveRate")), 400);
    assert_eq!(
        said(ask(None, EntryPath::UserRoute, "documents")).0,
        SIGN_IN
    );
}

#[test]
fn a_refusal_is_an_error_whose_text_is_the_sentence() {
    fn is_error<E: std::error::Error + Clone + PartialEq + Send + Sync + 'static>(_: &E) {}
    let e = ask(None, EntryPath::Rpc, "adminSnapshot").expect_err("nobody is signed in");
    is_error(&e);
    assert_eq!(e.to_string(), "กรุณาเข้าสู่ระบบ");
    assert_eq!(format!("{e}"), SIGN_IN);
    assert_eq!(e.clone(), e);
    assert_ne!(
        e,
        ask(Some(&front()), EntryPath::OwnerRoute, "list").expect_err("not the owner")
    );
}

#[test]
fn an_action_is_its_v2_name_and_nothing_else() {
    assert_eq!(action("saveTransaction").as_str(), "saveTransaction");
    assert_eq!(action("").as_str(), "");
    assert_eq!(
        action("adminSnapshot"),
        Action::new(String::from("adminSnapshot"))
    );
    assert_ne!(action("adminSnapshot"), action("adminsnapshot"));
}
