//! Who may call which kiosk action: the kiosk's role table, in its one home.
//!
//! Provided with ticket POL (design `docs/design/xchange-v3.md` §5.5.7). Written before the
//! implementation. Do not edit, delete or weaken.
//!
//! Until POL the table had two homes: `Policy` (path `Kiosk`, asked by no route) and
//! `KioskMachine::authorize` in `xc-kiosk` (asked by the kiosk route and again by the service).
//! POL leaves `Policy` alone; the role cases of `crates/xc-kiosk/tests/kiosk_machine_test.rs`
//! and `kiosk_service_test.rs` moved here.
//!
//! The answers are v2's, from the 10-06 bundle (`XC-Desk-v2-Passport-Drawer-20261006.zip`,
//! sha256 `6514b4c83c74916edba23d48e6d650051babe567c71e98bc84e7e0574375f570`; its `server.mjs`
//! has sha256 `99ddcd107bd4107226ce3065efcefea11a8b2a58d86a45b9a5872e571a8b1b0b`), function
//! `action2` (`server.mjs:7941-8058`):
//! - `manage`, `configure` — OWNER and RATES; anybody else, nobody included, "Owner or Rates
//!   access required." (`:7944-7945`);
//! - `rates`, `quote`, `confirm`, `status`, `customerReceipt` — nobody's role is asked;
//! - `notifications` — anybody signed in (`:8011-8012`);
//! - `list` — anybody signed in (`:8026`). **The 10-06 change**: the 10-05 bundle let only the
//!   desk list (`server.mjs:8012` and `:8036` there);
//! - every other name — OWNER, FRONT, BACKEND, MANAGER, else "Staff access required." (`:8027`);
//!   then `identityChecked`, `publishReceipt`, `get`, `accept` are answered and any other name
//!   is "Unknown kiosk action." (`:8057`).
//!
//! The same answers are recorded, caller by caller, in `fixtures/v2_policy_cases.json` (path
//! `Kiosk`, written by `api/tools/v2-fixtures/gen_policy.mjs` from that bundle); the last test
//! holds the table below to that recording.
//!
//! Moved by the author of OR-4 (#550, `docs/design/blueprint/org-roles-branches.md` §4.2, §4.3)
//! and restated for §11.2 (`docs/design/blueprint/user-management-rbac.md` §11.2, S11):
//! each v2 caller is now the user B1 makes of it (§3.3; OWNER → EXECUTIVE + ADMIN), and the
//! table's columns keep v2's names so the rows read as before. Under §11.2, `QueueOperate`
//! (CUSTOMERS CREATE) is held by EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER, TELLER (superseding C8):
//! OWNER and RATES are allowed on the desk row and hear "Unknown kiosk action." on an unknown name.
//! The last test holds the table to `fixtures/blueprint_policy_cases.json`.

mod policy_common;

use policy_common::*;
use xc_auth::domain::access::role_grants::{Group, RoleGrantsSnapshot};

/// The eleven callers v2 knows, in the order of the table's columns.
const CALLERS: [&str; 11] = [
    "ANON",
    "OWNER",
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

/// What a caller is told. `Ok` = let through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Said {
    Ok,
    /// R14 "Owner or Rates access required."
    OwnerOrRates,
    /// R15 "Staff access required."
    Staff,
    /// R16 "Unknown kiosk action."
    Unknown,
}

use Said::{OwnerOrRates as R14, Staff as R15, Unknown as R16};

const OK: Said = Said::Ok;

/// Names v2 does not answer: near misses of real ones, names of other paths, nothing at all.
const UNKNOWN_NAMES: [&str; 9] = [
    "nope",
    "",
    "List",
    "list ",
    " list",
    "Manage",
    "listing",
    "adminSnapshot",
    "opsClock",
];

/// The table: one row per action, one column per caller (`CALLERS`).
fn table() -> Vec<(&'static str, [Said; 11])> {
    //             ANON OWNER FRONT BACK  MGR  RATES MON  FIN  SYS  DOCS DREV
    let open = [OK, OK, OK, OK, OK, OK, OK, OK, OK, OK, OK];
    let signed_in = [R15, OK, OK, OK, OK, OK, OK, OK, OK, OK, OK];
    // §11.2 (S11, supersedes C8): QueueOperate (CUSTOMERS CREATE) is held by EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER, TELLER
    let desk = [R15, OK, OK, OK, OK, OK, R15, R15, R15, R15, R15];
    let currencies = [R14, OK, R14, R14, R14, OK, R14, R14, R14, R14, R14];
    // §11.2 (S11, supersedes C8): EXECUTIVE (OWNER) and RATE_MANAGER (RATES) hold QueueOperate, so both hear R16
    let unknown = [R15, R16, R16, R16, R16, R16, R15, R15, R15, R15, R15];
    let mut rows = vec![
        ("rates", open),
        ("quote", open),
        ("confirm", open),
        ("status", open),
        ("customerReceipt", open),
        ("notifications", signed_in),
        ("list", signed_in),
        ("accept", desk),
        ("get", desk),
        ("identityChecked", desk),
        ("publishReceipt", desk),
        ("manage", currencies),
        ("configure", currencies),
        // Amended by the author of KC-S #4176 (claude/studio-claude-10, `kiosk-cancel.md` §3.4):
        // staff cancel is the desk's `QueueOperate`; the customer's cancel is public, guarded by
        // its token, address and rate limit in the route.
        ("cancel", desk),
        ("customerCancel", open),
    ];
    rows.extend(UNKNOWN_NAMES.map(|name| (name, unknown)));
    rows
}

/// v2's caller as the user B1 makes of it (§3.3).
fn caller_of(name: &str) -> Option<Actor> {
    match name {
        "ANON" => None,
        "OWNER" => Some(owner()),
        v2 => Some(user(
            &format!("U-{v2}"),
            b1(v2).iter().map(|r| row(*r, Some("B2"))).collect(),
        )),
    }
}

fn expected(said: Said) -> Result<(), (String, u16)> {
    match said {
        Said::Ok => Ok(()),
        Said::OwnerOrRates => Err((KIOSK_OWNER_OR_RATES.to_owned(), 400)),
        Said::Staff => Err((KIOSK_STAFF.to_owned(), 400)),
        Said::Unknown => Err((KIOSK_UNKNOWN.to_owned(), 400)),
    }
}

#[test]
fn every_kiosk_action_by_every_caller_is_answered_from_the_one_table() {
    let mut wrong = Vec::new();
    for (name, row) in table() {
        for (who, said) in CALLERS.iter().zip(row) {
            let got = answer(ask(caller_of(who).as_ref(), EntryPath::Kiosk, name));
            if got != expected(said) {
                wrong.push(format!("{name:?} by {who}: want {said:?}, got {got:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} answers differ from the table:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_queue_is_listed_to_anybody_signed_in_since_the_10_06_bundle() {
    // server.mjs:8026 of the 10-06 bundle: `if (name === "list" && user) return queueWork.list(…)`
    assert_eq!(ask(Some(&owner()), EntryPath::Kiosk, "list"), Ok(()));
    for r in BLUEPRINT {
        assert_eq!(
            ask(Some(&staff(r, "B2")), EntryPath::Kiosk, "list"),
            Ok(()),
            "{}",
            r.as_str()
        );
    }
    assert_eq!(
        said(ask(None, EntryPath::Kiosk, "list")),
        (KIOSK_STAFF.to_owned(), 400)
    );
    // §11.2: QueueOperate is held by EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER, TELLER
    let off_desk = [
        Role::Admin,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ]
    .map(|r| staff(r, "B2"));
    for who in &off_desk {
        assert_eq!(
            said(ask(Some(who), EntryPath::Kiosk, "accept")),
            (KIOSK_STAFF.to_owned(), 400),
            "{:?}",
            who.roles
        );
    }
}

#[test]
fn a_name_v2_does_not_answer_is_refused_by_what_the_caller_is() {
    // nobody signed in: the staff sentence, never the unknown-action one
    for name in UNKNOWN_NAMES {
        assert_eq!(
            said(ask(None, EntryPath::Kiosk, name)),
            (KIOSK_STAFF.to_owned(), 400),
            "{name:?} by nobody"
        );
    }
    // §11.2 (S11, supersedes C8): QueueOperate holders (EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER,
    // TELLER, and the owner's user through EXECUTIVE): the name is unknown
    for name in UNKNOWN_NAMES {
        assert_eq!(
            said(ask(Some(&owner()), EntryPath::Kiosk, name)),
            (KIOSK_UNKNOWN.to_owned(), 400),
            "{name:?} by owner"
        );
    }
    for r in [
        Role::Executive,
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
    ] {
        for name in UNKNOWN_NAMES {
            assert_eq!(
                said(ask(Some(&staff(r, "B2")), EntryPath::Kiosk, name)),
                (KIOSK_UNKNOWN.to_owned(), 400),
                "{name:?} by {}",
                r.as_str()
            );
        }
    }
    // anybody signed in outside QueueOperate: refused as staff before the name is looked at
    let off_desk = [
        Role::Admin,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ]
    .map(|r| staff(r, "B2"));
    for who in &off_desk {
        for name in UNKNOWN_NAMES {
            assert_eq!(
                said(ask(Some(who), EntryPath::Kiosk, name)),
                (KIOSK_STAFF.to_owned(), 400),
                "{name:?} by {:?}",
                who.roles
            );
        }
    }
}

#[test]
fn the_table_is_what_the_mapping_records() {
    let f = mapped_fixture();
    let rows: Vec<_> = f.cases.iter().filter(|c| c.path == "Kiosk").collect();
    // 11 callers × the 13 names of `action2` and "nope"
    assert_eq!(rows.len(), 11 * 14);
    let table = table();
    for c in rows {
        let (_, row) = table
            .iter()
            .find(|(name, _)| *name == c.action)
            .unwrap_or_else(|| panic!("{:?} is in the table", c.action));
        let column = CALLERS
            .iter()
            .position(|who| *who == c.actor)
            .unwrap_or_else(|| panic!("{} is a caller of the table", c.actor));
        assert_eq!(
            expected(row[column]),
            recorded_mapped(c),
            "{:?} by {}",
            c.action,
            c.actor
        );
    }
}

#[test]
fn a_custom_group_holding_queue_operate_hears_kiosk_unknown_on_an_unknown_name() {
    struct ResetSnapshot;
    impl Drop for ResetSnapshot {
        fn drop(&mut self) {
            RoleGrants::install(RoleGrantsSnapshot::seeded());
        }
    }
    let _reset = ResetSnapshot;

    let mut groups: Vec<(Group, Vec<Permission>)> = RoleGrants::BUILT_IN
        .into_iter()
        .map(|r| (Group::built_in(r), RoleGrants::seed(r)))
        .collect();
    groups.push((
        Group {
            code: "KIOSK_DESK_CUSTOM".to_owned(),
            name: "Kiosk Desk Custom".to_owned(),
            base: Role::Admin,
            built_in: false,
        },
        vec![Permission::QueueOperate],
    ));
    RoleGrants::install(RoleGrantsSnapshot::new(1, groups));

    let custom_caller = user(
        "U-CUSTOM",
        vec![ScopedRole::in_group(
            "KIOSK_DESK_CUSTOM",
            Role::Admin,
            Some(branch("B2")),
            since(),
            None,
        )],
    );
    assert_eq!(
        ask(Some(&custom_caller), EntryPath::Kiosk, "nope"),
        Err(PolicyError::KioskUnknown)
    );
}
