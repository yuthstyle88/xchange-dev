//! OR-7.4 (#1023): custody stops naming v3's old identity types, and an employee's part in custody
//! stays exactly what it is today.
//!
//! 🔴 **Up-front tests by the ticket author (claude/dell-claude-2), re-cut on `stack/626-base`
//! `a3897cf5` (the #626 batch), building on claude/d7aa64's `4083143f`. Do not edit, delete or
//! weaken.**
//!
//! On `a3897cf5`, per the BE controller's ruling A (#1023 issuecomment after 6244788920):
//! - `custody_role.rs` is the only scope file that still names an old type (`Role`, `:8`);
//! - `custody_role_of(Role)` survives only to fill `CustodyActor.role`
//!   (`custody_save_step.rs:50-54`), a field nothing reads (`check_save_in`,
//!   `xc-shift/src/application/custody/service.rs:122-147`), so the field and the function go;
//! - `employee_custody_role(stored)` stays: `PgCustodyPorts::employee_role`
//!   (`infrastructure/custody/custody_ports.rs:167`) answers `BranchDayService`, which records the
//!   role on every attendance and event. It keeps every answer, through a stored-text table.
//!
//! The stored-text rows are `custody_role_test.rs`'s, restated with no old type. No database.

use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xchange::application::custody_role::employee_custody_role;

// ------------------------------------------------------------ no old identity types

/// #1023's file scope (§5) that can name an old type: the four files of the first cut, and the
/// save step ruling A adds. A file that no longer exists is skipped.
const SCOPE: [&str; 5] = [
    "src/application/custody_role.rs",
    "src/application/custody/employee_custody.rs",
    "src/infrastructure/custody/employee_custody.rs",
    "src/application/compat/actions/custody.rs",
    "src/application/workflows/custody_save_step.rs",
];

/// `line` without its `//` comment, leaving a `//` inside a string literal
/// (as `or7_5_save_actor_test.rs`).
fn strip_line_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        if bytes[i] == b'\\' {
            escaped = true;
            continue;
        }
        if bytes[i] == b'"' {
            in_quote = !in_quote;
        } else if !in_quote && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
    }
    line
}

/// Whether `line` (code, not comment) names one of the old identity types through its module:
/// `policy::value_objects::actor…`, `::role…`, `::actor_kind…`, or a `{…}` group that holds one.
/// Done criterion 3's grep, plus the grouped form #1023's own measure command looks for.
fn names_an_old_type(line: &str) -> bool {
    let code = strip_line_comment(line);
    let Some(at) = code.find("policy::value_objects::") else {
        return false;
    };
    let rest = &code[at + "policy::value_objects::".len()..];
    if rest.starts_with("actor") || rest.starts_with("role") {
        return true;
    }
    rest.starts_with('{') && (rest.contains("actor") || rest.contains("role"))
}

/// Every line of the in-scope files that still exist and name an old type, as `path:line: text`.
fn old_type_sites() -> (Vec<String>, usize) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    let mut present = 0;
    for rel in SCOPE {
        let path = root.join(rel);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        present += 1;
        for (n, line) in text.lines().enumerate() {
            if names_an_old_type(line) {
                hits.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    (hits, present)
}

#[test]
fn custody_names_none_of_the_old_identity_types() {
    let (hits, present) = old_type_sites();
    assert!(
        present > 0,
        "none of #1023's four files exists: the scope moved, re-measure the ticket"
    );
    assert!(
        hits.is_empty(),
        "custody uses access::Actor and its own role table, never xc_auth's old Actor / Role / \
         ActorKind (#1023 done criterion 3). {} site(s):\n{}",
        hits.len(),
        hits.join("\n")
    );
}

/// The scan is not blind: it finds each form criterion 3 and the ticket's measure command look
/// for, and lets comments and the new types through.
#[test]
fn the_scan_is_not_blind() {
    for found in [
        "use xc_auth::domain::policy::value_objects::actor::Actor;",
        "use xc_auth::domain::policy::value_objects::role::Role;",
        "use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;",
        "use xc_auth::domain::policy::value_objects::{actor::Actor, entry_path::EntryPath};",
        "    actor: &xc_auth::domain::policy::value_objects::actor::Actor,",
    ] {
        assert!(names_an_old_type(found), "missed: {found}");
    }
    for passed in [
        "use xc_auth::domain::access::Actor;",
        "use xc_auth::domain::access::Role;",
        "use xc_auth::domain::policy::value_objects::entry_path::EntryPath;",
        "// was xc_auth::domain::policy::value_objects::role::Role",
        "use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;",
    ] {
        assert!(!names_an_old_type(passed), "flagged: {passed}");
    }
}

// ------------------------------------------------------------ an employee's part in custody

/// Every staff text as stored keeps its custody part (`custody_role_test.rs`
/// `every_staff_text_as_stored_is_pinned_by_name`, without `Role::STAFF`).
#[test]
fn every_staff_text_as_stored_keeps_its_custody_part() {
    for (stored, want) in [
        ("FRONT", CustodyRole::Front),
        ("BACKEND", CustodyRole::Backend),
        ("MANAGER", CustodyRole::Manager),
        ("RATES", CustodyRole::Other),
        ("MONITOR", CustodyRole::Monitor),
        ("FINANCE", CustodyRole::Finance),
        ("SYSTEM", CustodyRole::Other),
        ("DOCUMENTS", CustodyRole::Other),
        ("DOCUMENT_REVIEW", CustodyRole::Other),
    ] {
        assert_eq!(employee_custody_role(stored), want, "{stored}");
    }
}

/// An employee is never the owner, whatever the row says (`an_employee_is_never_the_owner`).
#[test]
fn an_employee_row_is_never_the_owner() {
    assert_eq!(employee_custody_role("OWNER"), CustodyRole::Other);
}

/// The stored text is compared exactly: lower case, spaces, the empty text and a role that does
/// not exist are `Other` (`lower_case_is_not_a_role_text`,
/// `anything_not_exactly_a_staff_role_text_is_other`).
#[test]
fn anything_not_exactly_a_staff_text_is_other() {
    for stored in [
        "front",
        "Front",
        "",
        " FRONT",
        "FRONT ",
        "CASHIER",
        "TELLER",
        "BRANCH_MANAGER",
    ] {
        assert_eq!(
            employee_custody_role(stored),
            CustodyRole::Other,
            "{stored:?}"
        );
    }
}

// ------------------------------------------------------------ ruling A: the dead field goes

/// The text of `CustodyActor`'s definition, from `struct CustodyActor` to its closing `}`.
fn custody_actor_struct() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/xc-shift/src/domain/custody/value_objects/custody_actor.rs");
    let text = std::fs::read_to_string(&path).expect("custody_actor.rs exists");
    let at = text
        .find("pub struct CustodyActor")
        .expect("CustodyActor is defined in custody_actor.rs");
    let end = text[at..].find('}').expect("the struct closes") + at;
    text[at..=end].to_string()
}

/// Ruling A: `CustodyActor.role` is removed. Nothing reads it — `check_save_in` decides on
/// `employee_id` / `id` alone — so a value in it could only be a guess.
#[test]
fn custody_actor_has_no_role_field() {
    let def = custody_actor_struct();
    assert!(
        def.contains("pub id: String") && def.contains("pub employee_id: Option<String>"),
        "CustodyActor keeps who saves:\n{def}"
    );
    assert!(
        !def.lines()
            .map(strip_line_comment)
            .any(|l| l.trim_start().starts_with("pub role")),
        "CustodyActor.role is removed (ruling A on #1023):\n{def}"
    );
}

/// Ruling A: `custody_role_of(Role)` is deleted with its only caller; `custody_role.rs` keeps
/// `employee_custody_role` and nothing that takes the old `Role`.
#[test]
fn custody_role_of_is_gone_and_employee_custody_role_stays() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/custody_role.rs");
    let text = std::fs::read_to_string(&path).expect("custody_role.rs exists");
    let code: String = text
        .lines()
        .map(strip_line_comment)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("fn custody_role_of"),
        "custody_role_of(Role) is deleted (ruling A on #1023)"
    );
    assert!(
        code.contains("pub fn employee_custody_role(stored: &str) -> CustodyRole"),
        "employee_custody_role keeps its signature"
    );
}
