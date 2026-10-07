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

mod policy_common;

use policy_common::*;

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
    let desk = [R15, OK, OK, OK, OK, R15, R15, R15, R15, R15, R15];
    let currencies = [R14, OK, R14, R14, R14, OK, R14, R14, R14, R14, R14];
    let unknown = [R15, R16, R16, R16, R16, R15, R15, R15, R15, R15, R15];
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
    ];
    rows.extend(UNKNOWN_NAMES.map(|name| (name, unknown)));
    rows
}

fn caller_of(name: &str) -> Option<Actor> {
    match name {
        "ANON" => None,
        "OWNER" => Some(owner()),
        role_name => Some(staff(role(role_name), "B2")),
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
    for r in Role::STAFF {
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
    // taking a request off the queue stays the desk's
    for r in [
        Role::Rates,
        Role::Monitor,
        Role::Finance,
        Role::System,
        Role::Documents,
        Role::DocumentReview,
    ] {
        assert_eq!(
            said(ask(Some(&staff(r, "B2")), EntryPath::Kiosk, "accept")),
            (KIOSK_STAFF.to_owned(), 400),
            "{}",
            r.as_str()
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
    // the owner and the desk: the name is unknown
    let desk = [
        owner(),
        staff(Role::Front, "B2"),
        staff(Role::Backend, "B2"),
        staff(Role::Manager, "B2"),
    ];
    for who in &desk {
        for name in UNKNOWN_NAMES {
            assert_eq!(
                said(ask(Some(who), EntryPath::Kiosk, name)),
                (KIOSK_UNKNOWN.to_owned(), 400),
                "{name:?} by {}",
                who.role().as_str()
            );
        }
    }
    // an employee outside the desk: refused as staff before the name is looked at
    for r in [
        Role::Rates,
        Role::Monitor,
        Role::Finance,
        Role::System,
        Role::Documents,
        Role::DocumentReview,
    ] {
        for name in UNKNOWN_NAMES {
            assert_eq!(
                said(ask(Some(&staff(r, "B2")), EntryPath::Kiosk, name)),
                (KIOSK_STAFF.to_owned(), 400),
                "{name:?} by {}",
                r.as_str()
            );
        }
    }
}

#[test]
fn the_table_is_what_v2_recorded() {
    let f = fixture();
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
            recorded(c),
            "{:?} by {}",
            c.action,
            c.actor
        );
    }
}
