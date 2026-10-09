//! Who may do what: the authorisation rule, with no database and no HTTP.
//!
//! Provided with ticket T4a; moved 1:1 onto the Blueprint by the author of OR-4 (#550, design
//! `docs/design/blueprint/org-roles-branches.md` §4.3, one row per old test). These tests are the
//! specification: make them pass, do not edit, delete or weaken them.
//!
//! `fixtures/v2_policy_cases.json` is what v2 answered when each caller asked for each action on
//! each of its entry paths. `fixtures/blueprint_policy_cases.json` is the same 1,425 questions
//! asked by the B1-mapped users (§3.3: OWNER → one user holding EXECUTIVE + ADMIN, FRONT →
//! TELLER, …), written by `api/tools/policy-map.mjs`: its answers are v2's except where a rule of
//! §4.2 (C1–C8; C6 and C9 select no v2 case) or D-4 names the case. The custody rows (path
//! Custody, the staff bridge's `ops…` names, `adminOpenShift`/`adminCloseShift`) are #582's: they
//! keep v2's answer there and are not replayed here (`policy_custody_v2_test.rs`).
//!
//! What moved, by the old test's line at `8234e77` (§4.3's table):
//! - kept: `:52` → `the_mapped_fixture_is_whole`; `:99` → `every_mapped_case_is_answered_as_mapped`
//!   and `the_mapping_changes_exactly_c1_to_c9`; `:128`, `:169`, `:301`, `:320`, `:337`, `:371`,
//!   `:382`, `:417`, `:481`, `:548`, `:562`, `:668`, `:784`, `:804`, `:818`;
//! - renamed to the mapped roles: `:590`, `:609`, `:627`, `:677`;
//! - changed: `:147` (D-4), `:189`, `:214`, `:235` (§02, §07, B1), `:264` + `:281` (D-4), `:293`,
//!   `:397` (C2, C3), `:437` (C1), `:462` (C4), `:646` (C5, C7);
//! - dropped: `:254` (§07 has no owner kind; `scope_rules_hold` covers the rest);
//! - moved to #582: `:310`, `:523`, `:720`, `:744`, `:762`, and the custody lines of `:417`,
//!   `:437`, `:481`.
//!
//! A changed answer in an assertion below names its rule (C1–C9 of §4.2, or D-4).

mod policy_common;

use std::collections::{BTreeMap, BTreeSet};

use policy_common::*;
use xc_auth::domain::access::scope_rule::ScopeRule;

const B2: &str = "B2";
const B9: &str = "B9";

fn front() -> Actor {
    staff(Role::Teller, B2)
}

/// Every caller: nobody, the owner's user, one single-role user per Blueprint role.
fn everyone() -> Vec<Option<Actor>> {
    let mut all = vec![None, Some(owner())];
    all.extend(BLUEPRINT.into_iter().map(|r| Some(staff(r, B2))));
    all
}

/// The Blueprint roles, each held alone, that `name` lets through on `on` (in §3.1's order).
fn allowed_roles(on: EntryPath, name: &str) -> Vec<&'static str> {
    BLUEPRINT
        .into_iter()
        .filter(|r| ask(Some(&staff(*r, B2)), on, name).is_ok())
        .map(Role::as_str)
        .collect()
}

fn mapped_rows(roles: &Option<Vec<RoleRow>>) -> Option<Vec<(String, Option<String>)>> {
    roles.as_ref().map(|rs| {
        rs.iter()
            .map(|r| (r.role.clone(), r.branch.clone()))
            .collect()
    })
}

type Outcome = (
    bool,
    Option<String>,
    Option<String>,
    Option<u16>,
    Option<String>,
    Option<Behind>,
);

fn v2_outcome(c: &Case) -> Outcome {
    (
        c.allowed,
        c.branch.clone(),
        c.error.clone(),
        c.status,
        c.redirect.clone(),
        c.behind.clone(),
    )
}

fn mapped_outcome(c: &Mapped) -> Outcome {
    (
        c.allowed,
        c.branch.clone(),
        c.error.clone(),
        c.status,
        c.redirect.clone(),
        c.behind.clone(),
    )
}

// ── the rules of §4.2, written out here, independent of the generator ────────

/// #582's rows (§4.2 "Custody ops*", "Rpc adminOpenShift, adminCloseShift").
fn is_custody(path: &str, name: &str) -> bool {
    path == "Custody"
        || (path == "StaffRpc" && name.starts_with("ops"))
        || (matches!(path, "Rpc" | "StaffRpc")
            && matches!(name, "adminOpenShift" | "adminCloseShift"))
}

/// Which rule of §4.2 names a main case of v2's fixture, if any.
fn rule_of(c: &Case) -> Option<&'static str> {
    let (p, a, who) = (c.path.as_str(), c.action.as_str(), c.actor.as_str());
    if is_custody(p, a) {
        return None;
    }
    if matches!(p, "StaffRpc" | "StaffUpload" | "StaffFile" | "StaffDesk") && who == "OWNER" {
        return Some("C1");
    }
    if p == "Rpc" && matches!(a, "reserveCustomer" | "saveTransaction") && who == "OWNER" {
        return Some("C2");
    }
    if p == "Rpc" && a == "adminMoveStock" && matches!(who, "OWNER" | "BACKEND" | "MANAGER") {
        return Some("C3");
    }
    if who == "SYSTEM"
        && ((p == "Rpc" && a == "adminCreateBranch")
            || (p == "OwnerRoute" && c.route == "POST /api/employees"))
    {
        return Some("C4");
    }
    if p == "PaymentMethods" && a == "save" && who == "SYSTEM" {
        return Some("C5");
    }
    if p == "Devices"
        && matches!(
            a,
            "process" | "retry" | "drawerObservation" | "printerOnline"
        )
        && who == "MANAGER"
    {
        return Some("C7");
    }
    if p == "Kiosk"
        && matches!(
            a,
            "identityChecked" | "publishReceipt" | "get" | "accept" | "nope"
        )
        && who == "OWNER"
    {
        return Some("C8");
    }
    None
}

/// What a named main case becomes under its rule. `system` is v2's answer to SYSTEM on the same
/// path and name (C1: where neither EXECUTIVE nor ADMIN holds the row's permission, the owner's
/// user hears what ADMIN hears).
fn becomes(rule: &str, c: &Case, system: &Case) -> Result<(), (String, u16)> {
    match rule {
        "C1" => {
            let passes = (c.path == "StaffRpc"
                && matches!(
                    c.action.as_str(),
                    "adminSnapshot" | "getConnectionStatus" | "adminSaveRate"
                ))
                || c.path == "StaffUpload";
            if passes { Ok(()) } else { recorded_v2(system) }
        }
        "C2" => Err((ROLE_NOT_ALLOWED_HERE.to_owned(), 400)),
        "C3" if c.actor == "OWNER" => Err((ROLE_NOT_ALLOWED_HERE.to_owned(), 400)),
        "C3" | "C4" | "C5" | "C7" => Ok(()),
        "C8" => Err((KIOSK_STAFF.to_owned(), 400)),
        other => panic!("no rule {other}"),
    }
}

// ── the fixture ──────────────────────────────────────────────────────────────

#[test]
fn the_mapped_fixture_is_whole() {
    let v2 = fixture();
    let m = mapped_fixture();
    assert_eq!(m.cases.len(), 1425);
    assert_eq!(m.cases.len(), m.counts.cases);
    assert_eq!(m.cases.len(), v2.cases.len());
    assert_eq!(m.branch_cases.len(), 285);
    assert_eq!(m.branch_cases.len(), m.counts.branch_cases);
    assert_eq!(m.branch_cases.len(), v2.branch_cases.len());
    assert_eq!(
        m.cases.iter().filter(|c| c.allowed).count(),
        m.counts.allowed
    );
    assert_eq!(
        m.cases.iter().filter(|c| !c.allowed).count(),
        m.counts.refused
    );
    assert_eq!(
        m.rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["C1", "C2", "C3", "C4", "C5", "C7", "C8", "D4"]
    );
    assert!(m.rules.iter().all(|r| !r.says.is_empty()));
    assert_eq!(m.no_case, ["C6", "C9"]);
    assert_eq!(
        (m.out_of_scope.error.as_str(), m.out_of_scope.status),
        (OUT_OF_SCOPE, 403)
    );

    // case i is v2's case i, asked by the B1-mapped user
    for (i, (c, v)) in m
        .cases
        .iter()
        .chain(&m.branch_cases)
        .zip(v2.cases.iter().chain(&v2.branch_cases))
        .enumerate()
    {
        assert_eq!(
            (&c.path, &c.route, &c.actor, &c.action, &c.own),
            (&v.path, &v.route, &v.actor, &v.action, &v.own),
            "case {i}"
        );
        let want: Option<Vec<(String, Option<String>)>> = match v.actor.as_str() {
            "ANON" => None,
            who => Some(
                b1(who)
                    .iter()
                    .map(|r| {
                        let on = if who == "OWNER" { None } else { v.own.clone() };
                        (r.as_str().to_owned(), on)
                    })
                    .collect(),
            ),
        };
        assert_eq!(
            mapped_rows(&c.roles),
            want,
            "case {i}: roles of {}",
            v.actor
        );
    }

    // a main case names the branch v2 ran it on: an employee's own (v2 replaced the name)
    for (c, v) in m.cases.iter().zip(&v2.cases) {
        let want = if v.own.is_some() {
            v.own.clone()
        } else {
            v.named.clone()
        };
        assert_eq!(c.named, want, "{} {:?} as {}", c.path, c.action, c.actor);
    }
    // a branch case names what v2's did
    for (c, v) in m.branch_cases.iter().zip(&v2.branch_cases) {
        assert_eq!(c.named, v.named);
    }

    // #582's rows, and only those, are marked and kept as v2 answered
    let custody: Vec<_> = m
        .cases
        .iter()
        .zip(&v2.cases)
        .filter(|(c, _)| c.owner.is_some())
        .collect();
    assert_eq!(custody.len(), m.counts.custody);
    assert_eq!(custody.len(), 402);
    for (c, v) in &custody {
        assert!(is_custody(&c.path, &c.action), "{} {:?}", c.path, c.action);
        assert_eq!(c.owner.as_deref(), Some("#582"));
        assert_eq!(mapped_outcome(c), v2_outcome(v));
        assert_eq!(c.change, None);
    }
    assert!(
        m.cases
            .iter()
            .filter(|c| c.owner.is_none())
            .all(|c| !is_custody(&c.path, &c.action))
    );

    // every path, and on every path somebody allowed and somebody refused
    for p in PATHS {
        let rows: Vec<_> = m.cases.iter().filter(|c| c.path == path_name(p)).collect();
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
}

#[test]
fn the_mapping_changes_exactly_c1_to_c9() {
    let v2 = fixture();
    let m = mapped_fixture();
    let mut per_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for (c, v) in m.cases.iter().zip(&v2.cases) {
        let label = format!("{} {:?} as {}", v.path, v.action, v.actor);
        let differs = mapped_outcome(c) != v2_outcome(v);
        match rule_of(v) {
            Some(rule) => {
                let system = v2
                    .cases
                    .iter()
                    .find(|s| s.path == v.path && s.action == v.action && s.actor == "SYSTEM")
                    .expect("SYSTEM asked every name");
                assert!(differs, "{label}: {rule} names it but nothing changed");
                assert_eq!(c.change.as_deref(), Some(rule), "{label}");
                assert_eq!(recorded_mapped(c), becomes(rule, v, system), "{label}");
                // a rule of §4.2 moves no branch: a refusal shows none, a call let through shows v2's
                let want_branch = if c.allowed && v.allowed {
                    v.branch.clone()
                } else {
                    None
                };
                assert_eq!(c.branch, want_branch, "{label}");
                *per_rule.entry(rule).or_default() += 1;
            }
            None => {
                assert!(!differs, "{label}: changed, but no rule of §4.2 names it");
                assert_eq!(c.change, None, "{label}");
            }
        }
    }
    assert_eq!(
        per_rule,
        BTreeMap::from([
            ("C1", 18),
            ("C2", 2),
            ("C3", 3),
            ("C4", 8),
            ("C5", 1),
            ("C7", 4),
            ("C8", 5),
        ])
    );
    assert_eq!(m.counts.changed, 41);

    // D-4 on the branch cases: an employee naming a branch not theirs is refused, except on the
    // public kiosk, where nobody's scope is asked and the call runs on the branch named
    let mut d4 = 0;
    for (c, v) in m.branch_cases.iter().zip(&v2.branch_cases) {
        let label = format!(
            "{} {:?} as {} naming {:?}",
            v.path, v.action, v.actor, v.named
        );
        let named_elsewhere = v.own.is_some() && v.named.is_some() && v.named != v.own;
        if named_elsewhere {
            d4 += 1;
            assert_eq!(c.change.as_deref(), Some("D4"), "{label}");
            if v.path == "Kiosk" && v.action == "rates" {
                assert!(c.allowed, "{label}");
                assert_eq!(c.branch, v.named, "{label}");
            } else {
                assert_eq!(
                    recorded_mapped(c),
                    Err((OUT_OF_SCOPE.to_owned(), 403)),
                    "{label}"
                );
            }
            assert_ne!(mapped_outcome(c), v2_outcome(v), "{label}");
        } else {
            assert_eq!(c.change, None, "{label}");
            assert_eq!(mapped_outcome(c), v2_outcome(v), "{label}");
        }
    }
    assert_eq!(d4, 85);
    assert_eq!(m.counts.branch_changed, 85);
}

#[test]
fn every_mapped_case_is_answered_as_mapped() {
    let m = mapped_fixture();
    let mut wrong = Vec::new();
    let mut asked_n = 0;
    for c in m
        .cases
        .iter()
        .chain(&m.branch_cases)
        .filter(|c| c.owner.is_none())
    {
        let who = caller(c.roles.as_deref());
        let name = asked(&c.route, &c.path, &c.action);
        let named = c.named.as_deref().map(branch);
        let got = answer(Policy::authorize(
            who.as_ref(),
            path(&c.path),
            &action(&name),
            named.as_ref(),
            now(),
        ));
        let want = recorded_mapped(c);
        if got != want {
            wrong.push(format!(
                "{} {:?} as {} ({}) naming {:?}: mapped {:?}, here {:?}",
                c.path, name, c.actor, c.route, c.named, want, got
            ));
        }
        asked_n += 1;
    }
    assert_eq!(asked_n, 1425 - 402 + 285);
    assert!(
        wrong.is_empty(),
        "{} of {} answers differ from the mapped fixture; the first ones:\n{}",
        wrong.len(),
        asked_n,
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
    // v2's 22 sentences, less the five only the custody engine says (#582), plus `outOfScope`
    let v2 = fixture();
    let custody_only: BTreeSet<&str> = [
        BACKEND_USES_COMMIT,
        EMPLOYEE_ACCOUNT,
        CLOCK_IN_FIRST,
        OWNER_ENABLES,
        SIGN_NOT_ALLOWED,
    ]
    .into();
    let mut want: BTreeSet<(String, u16)> = v2
        .refusals
        .iter()
        .filter(|r| !custody_only.contains(r.error.as_str()))
        .map(|r| (r.error.clone(), r.status))
        .collect();
    assert_eq!(v2.refusals.len(), 22);
    assert_eq!(want.len(), 17);
    want.insert((OUT_OF_SCOPE.to_owned(), 403));

    let m = mapped_fixture();
    let mut got = BTreeSet::new();
    for c in m
        .cases
        .iter()
        .chain(&m.branch_cases)
        .filter(|c| c.owner.is_none())
    {
        let who = caller(c.roles.as_deref());
        let named = c.named.as_deref().map(branch);
        if let Err(e) = Policy::authorize(
            who.as_ref(),
            path(&c.path),
            &action(&asked(&c.route, &c.path, &c.action)),
            named.as_ref(),
            now(),
        ) {
            got.insert((e.to_string(), e.status()));
        }
    }
    assert_eq!(got, want);
}

#[test]
fn every_call_runs_on_the_branch_named_in_scope() {
    // D-4 (§02 "Server ตรวจ … branch scope"): a branch outside the caller's scope is refused, no
    // longer replaced by the employee's own
    let m = mapped_fixture();
    let (mut ran, mut refused) = (0, 0);
    for c in m
        .cases
        .iter()
        .chain(&m.branch_cases)
        .filter(|c| c.owner.is_none())
    {
        let who = caller(c.roles.as_deref());
        let got = ask_on(
            who.as_ref(),
            path(&c.path),
            &asked(&c.route, &c.path, &c.action),
            c.named.as_deref(),
        );
        let label = format!(
            "{} {} as {} naming {:?}",
            c.route, c.action, c.actor, c.named
        );
        if let Some(want) = &c.branch {
            assert_eq!(got, Ok(branch(want)), "{label}");
            ran += 1;
        } else if c.change.as_deref() == Some("D4") {
            assert_eq!(said(got), (OUT_OF_SCOPE.to_owned(), 403), "{label}");
            refused += 1;
        }
    }
    assert_eq!((ran, refused), (456, 76));
}

#[test]
fn which_branch_an_employee_belongs_to_changes_no_answer() {
    let m = mapped_fixture();
    for c in m
        .cases
        .iter()
        .filter(|c| c.own.is_some() && c.owner.is_none())
    {
        for home in ["MAIN", B9, "B1700000000000-x"] {
            let rows: Vec<RoleRow> = c
                .roles
                .clone()
                .expect("an employee has roles")
                .into_iter()
                .map(|r| RoleRow {
                    branch: Some(home.to_owned()),
                    ..r
                })
                .collect();
            let who = caller(Some(&rows));
            assert_eq!(
                answer(ask_on(
                    who.as_ref(),
                    path(&c.path),
                    &asked(&c.route, &c.path, &c.action),
                    Some(home),
                )),
                recorded_mapped(c),
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
fn roles_are_spelled_as_the_blueprint_spells_them() {
    // §02, §15; B1. PARTNER is Broker-Platform's, not a v3 role.
    let spelled: Vec<&str> = BLUEPRINT.into_iter().map(Role::as_str).collect();
    assert_eq!(
        spelled,
        [
            "EXECUTIVE",
            "ADMIN",
            "RATE_MANAGER",
            "BRANCH_MANAGER",
            "TELLER",
            "FINANCE",
            "MARKETING",
            "CUSTOMER",
            "VERIFICATION_REVIEWER"
        ]
    );
    assert_eq!(spelled, mapped_fixture().roles);
    for r in BLUEPRINT {
        assert_eq!(Role::parse(r.as_str()), Some(r));
    }
}

#[test]
fn a_role_the_blueprint_does_not_have_is_no_role() {
    for text in [
        "",
        "INTERN",
        "teller",
        "Teller",
        " TELLER",
        "TELLER ",
        "BRANCH-MANAGER",
        "BRANCHMANAGER",
        "FRONT",
        "OWNER",
        "PARTNER",
        "SYSTEM",
        "MONITOR",
        "TELLER,ADMIN",
    ] {
        assert_eq!(Role::parse(text), None, "{text:?}");
    }
}

// ── who is calling ───────────────────────────────────────────────────────────

#[test]
fn scope_rules_hold() {
    // §3.2 (§07 UserRole, §02 scopes): EXECUTIVE organisation-wide; TELLER and BRANCH_MANAGER
    // exactly one branch; RATE_MANAGER as many as granted
    let b2 = branch(B2);
    let b9 = branch(B9);
    assert!(ScopeRule::check(Role::Executive, None, &[]).is_ok());
    assert!(ScopeRule::check(Role::Executive, Some(&b2), &[]).is_err());
    for r in [Role::Teller, Role::BranchManager] {
        assert!(ScopeRule::check(r, None, &[]).is_err(), "{}", r.as_str());
        assert!(
            ScopeRule::check(r, Some(&b2), &[]).is_ok(),
            "{}",
            r.as_str()
        );
        assert!(
            ScopeRule::check(r, Some(&b9), &[row(r, Some(B2))]).is_err(),
            "{} takes one branch",
            r.as_str()
        );
    }
    assert!(
        ScopeRule::check(
            Role::RateManager,
            Some(&b9),
            &[row(Role::RateManager, Some(B2))]
        )
        .is_ok()
    );
    // the owner's user is what B3 makes it
    let o = owner();
    assert_eq!(
        o.roles
            .iter()
            .map(|r| (r.role, r.branch.clone()))
            .collect::<Vec<_>>(),
        [(Role::Executive, None), (Role::Admin, None)]
    );
}

// ── the branch ───────────────────────────────────────────────────────────────

#[test]
fn naming_a_branch_outside_scope_is_refused() {
    // D-4: v2 overwrote the branch and went on; the Blueprint's server refuses it
    for r in BLUEPRINT
        .into_iter()
        .filter(|r| !matches!(r, Role::Executive | Role::Customer))
    {
        let e = staff(r, B2);
        let on = |named| ask_on(Some(&e), EntryPath::UserRoute, "desk", named);
        assert_eq!(
            said(on(Some(B9))),
            (OUT_OF_SCOPE.to_owned(), 403),
            "{}",
            r.as_str()
        );
        assert_eq!(
            said(on(Some("MAIN"))),
            (OUT_OF_SCOPE.to_owned(), 403),
            "{}",
            r.as_str()
        );
        assert_eq!(on(Some(B2)), Ok(branch(B2)), "{}", r.as_str());
        assert_eq!(on(None), Ok(branch(B2)), "{}", r.as_str());
    }
    // an employee of MAIN is held there too: MAIN is a branch, not a wildcard
    let main = staff(Role::BranchManager, "MAIN");
    assert_eq!(
        said(ask_on(Some(&main), EntryPath::UserRoute, "desk", Some(B9))),
        (OUT_OF_SCOPE.to_owned(), 403)
    );
    // the teller's own sale on either path, and the same sale named on another branch
    let t = front();
    for on in [EntryPath::Rpc, EntryPath::StaffRpc] {
        assert_eq!(
            ask_on(Some(&t), on, "saveTransaction", Some(B2)),
            Ok(branch(B2))
        );
        assert_eq!(
            said(ask_on(Some(&t), on, "saveTransaction", Some(B9))),
            (OUT_OF_SCOPE.to_owned(), 403)
        );
    }
    // a role the caller holds elsewhere is not one it holds here
    let rm = user(
        "U-RM",
        vec![
            row(Role::RateManager, Some(B2)),
            row(Role::RateManager, Some(B9)),
        ],
    );
    assert_eq!(
        ask_on(Some(&rm), EntryPath::Rpc, "rateDraft", Some(B9)),
        Ok(branch(B9))
    );
    assert_eq!(
        said(ask_on(Some(&rm), EntryPath::Rpc, "rateDraft", Some("B3"))),
        (OUT_OF_SCOPE.to_owned(), 403)
    );
}

#[test]
fn an_organisation_wide_user_works_on_the_branch_named_and_on_main_when_none_is() {
    // MAIN is v3's rule (`policy.rs:43-53`); the Blueprint is silent (§11)
    for who in [owner(), user("U-A", vec![row(Role::Admin, None)])] {
        let on = |named| ask_on(Some(&who), EntryPath::UserRoute, "desk", named);
        assert_eq!(on(Some(B9)), Ok(branch(B9)));
        assert_eq!(on(Some(B2)), Ok(branch(B2)));
        assert_eq!(on(None), Ok(branch("MAIN")));
    }
}

#[test]
fn a_caller_who_is_not_signed_in_is_on_the_branch_named() {
    // the customer kiosk: nobody is signed in and the screen says which branch it stands in
    assert_eq!(
        ask_on(None, EntryPath::Kiosk, "rates", Some(B9)),
        Ok(branch(B9))
    );
    assert_eq!(
        ask_on(None, EntryPath::Kiosk, "rates", None),
        Ok(branch("MAIN"))
    );
}

// ── the two RPC paths keep their own lists ───────────────────────────────────

#[test]
fn the_desk_configuration_depends_on_the_path() {
    // CompatRead on Rpc: every role but MARKETING and CUSTOMER
    assert_eq!(
        allowed_roles(EntryPath::Rpc, "adminDeskConfig"),
        [
            "EXECUTIVE",
            "ADMIN",
            "RATE_MANAGER",
            "BRANCH_MANAGER",
            "TELLER",
            "FINANCE",
            "VERIFICATION_REVIEWER"
        ]
    );
    // ExchangeTransact on the bridge
    assert_eq!(
        allowed_roles(EntryPath::StaffRpc, "adminDeskConfig"),
        ["TELLER"]
    );
    assert_eq!(
        said(ask(
            Some(&staff(Role::VerificationReviewer, B2)),
            EntryPath::StaffRpc,
            "adminDeskConfig"
        )),
        (FRONT_ONLY.to_owned(), 400)
    );
}

#[test]
fn reading_a_transactions_documents_depends_on_the_path() {
    // not on /api/rpc at all, for nobody's role, nor for the owner's user
    assert!(allowed_roles(EntryPath::Rpc, "documents").is_empty());
    assert_eq!(
        said(ask(Some(&owner()), EntryPath::Rpc, "documents")),
        (INVALID_REQUEST.to_owned(), 400)
    );
    // on the staff bridge: IdentityDocumentRead, for everybody v2 let in (B1) — not MARKETING (C6)
    assert_eq!(
        allowed_roles(EntryPath::StaffRpc, "documents"),
        [
            "BRANCH_MANAGER",
            "TELLER",
            "FINANCE",
            "VERIFICATION_REVIEWER"
        ]
    );
    for r in [Role::RateManager, Role::Admin] {
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
    // C6: MARKETING never opens identity documents (§02), on any of the three doors
    let mk = staff(Role::Marketing, B2);
    for (on, name) in [
        (EntryPath::StaffRpc, "documents"),
        (EntryPath::UserRoute, "documents"),
        (EntryPath::File, "file"),
        (EntryPath::StaffFile, "file"),
    ] {
        assert!(
            ask(Some(&mk), on, name).is_err(),
            "{} {name}",
            path_name(on)
        );
    }
}

#[test]
fn the_two_paths_refuse_in_sentences_one_word_apart() {
    let m = staff(Role::VerificationReviewer, B2);
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
        // C2: ExchangeTransact is TELLER's alone (§03 B03)
        assert_eq!(allowed_roles(on, "reserveCustomer"), ["TELLER"]);
        assert_eq!(allowed_roles(on, "saveTransaction"), ["TELLER"]);
        // RatesDraft: RATE_MANAGER, and EXECUTIVE per PR 570 A5 (on the bridge too: C1)
        assert_eq!(
            allowed_roles(on, "adminSaveRate"),
            ["EXECUTIVE", "RATE_MANAGER"]
        );
        assert_eq!(allowed_roles(on, "adminSnapshot").len(), 7);
        assert_eq!(allowed_roles(on, "getConnectionStatus").len(), 7);
    }
    // C4: BranchesManage is ADMIN's (§03 A04); the bridge has no row for it
    assert_eq!(
        allowed_roles(EntryPath::Rpc, "adminCreateBranch"),
        ["ADMIN"]
    );
    assert!(allowed_roles(EntryPath::StaffRpc, "adminCreateBranch").is_empty());
    // RequestAbandon (v3-only): EXECUTIVE, the B1 translation of OWNER
    assert_eq!(
        allowed_roles(EntryPath::Rpc, "adminAbandonRequest"),
        ["EXECUTIVE"]
    );
    assert!(allowed_roles(EntryPath::StaffRpc, "adminAbandonRequest").is_empty());
}

#[test]
fn moving_stock_is_branch_manager_and_finance() {
    // C3: CashManage (§03 B05, users MANAGER/FINANCE); was OWNER and FINANCE (`policy_test.rs:316`)
    assert_eq!(
        allowed_roles(EntryPath::Rpc, "adminMoveStock"),
        ["BRANCH_MANAGER", "FINANCE"]
    );
    assert!(allowed_roles(EntryPath::StaffRpc, "adminMoveStock").is_empty());
}

// ── the owner ────────────────────────────────────────────────────────────────

#[test]
fn the_owners_user_may_do_the_owner_rpc_actions_its_roles_allow() {
    let o = owner();
    for name in [
        "adminSnapshot",
        "adminDeskConfig",
        "adminSaveRate",
        "adminCreateBranch", // C4: kept through ADMIN
        "adminAbandonRequest",
        "getConnectionStatus",
    ] {
        assert_eq!(ask(Some(&o), EntryPath::Rpc, name), Ok(()), "{name}");
    }
    // C3 (adminMoveStock) and C2 (reserveCustomer, saveTransaction): EXECUTIVE is refused and
    // ADMIN holds neither
    for name in ["adminMoveStock", "reserveCustomer", "saveTransaction"] {
        assert_eq!(
            said(ask(Some(&o), EntryPath::Rpc, name)),
            (ROLE_NOT_ALLOWED_HERE.to_owned(), 400),
            "{name}"
        );
    }
    // EXECUTIVE alone, without ADMIN, does not create a branch (C4)
    let exec = staff(Role::Executive, B2);
    assert_eq!(
        said(ask(Some(&exec), EntryPath::Rpc, "adminCreateBranch")),
        (ROLE_NOT_ALLOWED_HERE.to_owned(), 400)
    );
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
        (EntryPath::OwnerRoute, "employees"),
        (EntryPath::OwnerRoute, "/api/reference-rates"),
        (EntryPath::OwnerRoute, "/api/bookings"),
        (EntryPath::UserRoute, "documents"),
    ] {
        assert_eq!(ask(Some(&o), on, name), Ok(()), "{} {name}", path_name(on));
    }
    // (the kiosk queue's `accept` left this list with C8: `the_kiosk_queue_is_for_teller_and_branch_manager`)
}

#[test]
fn a_staff_path_admits_whoever_holds_its_role() {
    // C1 (§07: one User, no owner kind). The owner's user is answered by its two roles: CompatRead
    // through ADMIN, RatesDraft and ProofUpload through EXECUTIVE; elsewhere it hears ADMIN's
    // refusal (`policy_test.rs:437` said EMPLOYEE_AUTH_REQUIRED to all of these)
    let o = owner();
    for (on, name) in [
        (EntryPath::StaffRpc, "adminSnapshot"),
        (EntryPath::StaffRpc, "getConnectionStatus"),
        (EntryPath::StaffRpc, "adminSaveRate"),
        (EntryPath::StaffUpload, "upload"),
    ] {
        assert_eq!(ask(Some(&o), on, name), Ok(()), "{} {name}", path_name(on));
    }
    for (on, name, sentence) in [
        (EntryPath::StaffRpc, "reserveCustomer", ROLE_NOT_ALLOWED),
        (EntryPath::StaffRpc, "adminDeskConfig", FRONT_ONLY),
        (EntryPath::StaffRpc, "documents", DOCUMENTS_NOT_READABLE),
        (EntryPath::StaffFile, "file", FILES_NOT_READABLE),
        (EntryPath::StaffDesk, "desk", FRONT_ONLY),
    ] {
        assert_eq!(
            said(ask(Some(&o), on, name)),
            (sentence.to_owned(), 400),
            "{} {name}",
            path_name(on)
        );
    }
    // an organisation-wide ADMIN alone passes the bridge's reads as the owner does
    let admin = user("U-A", vec![row(Role::Admin, None)]);
    assert_eq!(
        ask(Some(&admin), EntryPath::StaffRpc, "adminSnapshot"),
        Ok(())
    );
    // and the owner RPC still has no custody action at all
    assert_eq!(
        said(ask(Some(&o), EntryPath::Rpc, "opsEnable")),
        (INVALID_REQUEST.to_owned(), 400)
    );
}

#[test]
fn only_admin_passes_branch_and_user_management() {
    // C4 (§03 A04/A05 user ADMIN; §02 ADMIN "ผู้ใช้ บทบาท สาขา"): UsersManage on the employees route
    // (it asks as "employees"; the view `admin_employees` as "list")
    for name in ["employees", "list"] {
        assert_eq!(ask(Some(&owner()), EntryPath::OwnerRoute, name), Ok(()));
        assert_eq!(
            allowed_roles(EntryPath::OwnerRoute, name),
            ["ADMIN"],
            "{name}"
        );
        for r in BLUEPRINT.into_iter().filter(|r| *r != Role::Admin) {
            assert_eq!(
                said(ask(Some(&staff(r, B2)), EntryPath::OwnerRoute, name)),
                (OWNER_ONLY.to_owned(), 403),
                "{} {name}",
                r.as_str()
            );
        }
        assert_eq!(
            said(ask(None, EntryPath::OwnerRoute, name)),
            (SIGN_IN.to_owned(), 401)
        );
    }
}

// ── names the route table does not know ──────────────────────────────────────

#[test]
fn an_unknown_action_is_refused_in_the_words_of_its_path() {
    let m = staff(Role::BranchManager, B2);
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
    // UserRoute left this list with C6: `documents` is DocumentsList, which MARKETING lacks, and
    // the other pages are SignedIn
    let one_purpose = [
        (EntryPath::Upload, "upload"),
        (EntryPath::StaffUpload, "upload"),
        (EntryPath::File, "file"),
        (EntryPath::StaffFile, "file"),
        (EntryPath::StaffDesk, "desk"),
        (EntryPath::OwnerRoute, "employees"),
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
fn teller_and_finance_upload_evidence_and_nobody_else_on_the_staff() {
    // ProofUpload: TELLER, FINANCE, and EXECUTIVE (the B1 translation of OWNER on /api/upload;
    // on the staff path too since C1)
    for on in [EntryPath::Upload, EntryPath::StaffUpload] {
        assert_eq!(
            allowed_roles(on, "upload"),
            ["EXECUTIVE", "TELLER", "FINANCE"]
        );
        assert_eq!(
            said(ask(Some(&staff(Role::BranchManager, B2)), on, "upload")),
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
fn rate_managers_and_admins_read_no_customer_file_and_each_path_says_so_differently() {
    for r in [Role::RateManager, Role::Admin] {
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
    // FileRead: everybody v2 let in, B1-mapped, less MARKETING (C6)
    assert_eq!(
        allowed_roles(EntryPath::File, "file"),
        [
            "EXECUTIVE",
            "BRANCH_MANAGER",
            "TELLER",
            "FINANCE",
            "VERIFICATION_REVIEWER"
        ]
    );
    // IdentityDocumentRead on the staff path
    assert_eq!(
        allowed_roles(EntryPath::StaffFile, "file"),
        [
            "BRANCH_MANAGER",
            "TELLER",
            "FINANCE",
            "VERIFICATION_REVIEWER"
        ]
    );
}

#[test]
fn the_staff_desk_page_is_tellers() {
    assert_eq!(allowed_roles(EntryPath::StaffDesk, "desk"), ["TELLER"]);
    assert_eq!(
        said(ask(
            Some(&staff(Role::BranchManager, B2)),
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
        ["EXECUTIVE", "BRANCH_MANAGER", "TELLER"]
    );
    // C7: ex-MANAGER gains them with ex-BACKEND (B1 merges the two)
    for name in ["process", "retry", "drawerObservation"] {
        assert_eq!(
            allowed_roles(EntryPath::Devices, name),
            ["EXECUTIVE", "BRANCH_MANAGER", "TELLER"]
        );
    }
    assert_eq!(
        allowed_roles(EntryPath::Devices, "printerOnline"),
        ["EXECUTIVE", "BRANCH_MANAGER"]
    );
    assert_eq!(
        said(ask(Some(&front()), EntryPath::Devices, "printerOnline")),
        (DEVICE_STEP_NOT_ALLOWED.to_owned(), 400)
    );
    // C5: SystemSettings is EXECUTIVE and ADMIN (§02 ADMIN "ตั้งค่าระบบ")
    for name in ["setPrinter", "setDrawer"] {
        assert_eq!(
            allowed_roles(EntryPath::Devices, name),
            ["EXECUTIVE", "ADMIN"]
        );
    }
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
fn the_kiosk_queue_is_for_teller_and_branch_manager() {
    // C8: QueueOperate (§03 B06, users TELLER/MANAGER); EXECUTIVE is refused
    for name in ["accept", "get", "identityChecked", "publishReceipt"] {
        assert_eq!(
            allowed_roles(EntryPath::Kiosk, name),
            ["BRANCH_MANAGER", "TELLER"]
        );
        assert_eq!(
            said(ask(Some(&owner()), EntryPath::Kiosk, name)),
            (KIOSK_STAFF.to_owned(), 400)
        );
        assert_eq!(
            said(ask(None, EntryPath::Kiosk, name)),
            (KIOSK_STAFF.to_owned(), 400)
        );
    }
    // the waiting count and the queue's list, to anybody signed in
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
    // which currencies the kiosk offers: RatesDraft
    for name in ["manage", "configure"] {
        assert_eq!(
            allowed_roles(EntryPath::Kiosk, name),
            ["EXECUTIVE", "RATE_MANAGER"]
        );
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

// ── the refusal ──────────────────────────────────────────────────────────────

#[test]
fn a_refusal_carries_the_status_v2_answered_with() {
    let m = staff(Role::VerificationReviewer, B2);
    let status = |got: Result<(), PolicyError>| said(got).1;
    assert_eq!(status(ask(None, EntryPath::Rpc, "adminSnapshot")), 401);
    assert_eq!(status(ask(None, EntryPath::UserRoute, "documents")), 401);
    assert_eq!(status(ask(None, EntryPath::File, "file")), 401);
    assert_eq!(status(ask(None, EntryPath::StaffRpc, "adminSnapshot")), 401);
    assert_eq!(
        status(ask(Some(&m), EntryPath::OwnerRoute, "employees")),
        403
    );
    assert_eq!(
        status(ask(Some(&staff(Role::Admin, B2)), EntryPath::File, "file")),
        404
    );
    assert_eq!(status(ask(Some(&m), EntryPath::Rpc, "adminSaveRate")), 400);
    assert_eq!(
        said(ask(None, EntryPath::UserRoute, "documents")).0,
        SIGN_IN
    );
    // the one new refusal (§10 `outOfScope`)
    assert_eq!(
        (
            PolicyError::OutOfScope.to_string(),
            PolicyError::OutOfScope.status()
        ),
        (OUT_OF_SCOPE.to_owned(), 403)
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
        ask(Some(&front()), EntryPath::OwnerRoute, "employees").expect_err("not ADMIN")
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
