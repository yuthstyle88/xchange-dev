//! OR-4b (#767, #550): guards that keep every route on the route table once the routes ask
//! `Policy::authorize`.
//!
//! Written before the implementation by the ticket author (design
//! `docs/design/blueprint/org-roles-branches.md` §4.1, §4.2; the lane controller's rulings (a) and
//! (b) on #767; the note on `Target.customer_id` on #767). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! - **Ruling (b):** a v3 view, action or command file answers through the route table. It declares
//!   `INHERITS` (resolved by `Policy::need`) on an impl of the shared trait, or it declares the
//!   explicit public marker `PUBLIC_ROUTE` instead. A file that does neither could register a
//!   handler that skips the table.
//! - **Ruling (a):** `legacy_actor` stays only for downstream business services. No result of it
//!   reaches `Policy::*`, and the v2 rule (`Policy::allow`, `Policy::branch`) is asked only on the
//!   custody rows (`Need::Custody`, #582).
//! - **`Target.customer_id`:** `Policy::authorize` builds its `Target` with the actor's own
//!   `customer_id` (`xc-auth` `policy.rs`), so a `CustomerSelf` row would compare the actor with
//!   itself and always pass. Until a route can name the customer it targets, no row asks
//!   `CustomerSelf`; and a target that names no customer never passes `CustomerSelf`.
//!
//! No database.

mod or4_route_rules;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use serde_json::Value;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Refusal, Role, ScopedRole, Target};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

fn api_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// The text of a file without its comment lines.
fn code_of(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn relative(path: &Path) -> String {
    path.strip_prefix(api_dir())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The files `build.rs` registers as v3 routes: every `.rs` under `views/`, `actions/` and
/// `commands/` but a `mod.rs`.
fn v3_route_files() -> Vec<PathBuf> {
    let v3 = api_dir().join("src/presentation/http/v3");
    let mut files = Vec::new();
    for dir in ["views", "actions", "commands"] {
        rust_files(&v3.join(dir), &mut files);
    }
    files.retain(|f| f.file_name().is_some_and(|n| n != "mod.rs"));
    files.sort();
    files
}

#[test]
fn every_v3_route_file_declares_inherits_or_the_public_marker() {
    // ruling (b) on #767: INHERITS stays, resolved through the route table (§4.1, amended 2026-10-10)
    let files = v3_route_files();
    assert!(
        files.len() >= 40,
        "the v3 route files were found: {}",
        files.len()
    );
    let mut neither = Vec::new();
    let mut inherits_off_the_trait = Vec::new();
    for file in &files {
        let code = code_of(file);
        if let Err(why) = or4_route_rules::route_file_verdict(&code) {
            if why.contains("INHERITS") && why.contains("outside") {
                inherits_off_the_trait.push(relative(file));
            } else {
                neither.push(relative(file));
            }
        }
    }
    assert_eq!(
        neither,
        Vec::<String>::new(),
        "a v3 route file declares neither INHERITS nor PUBLIC_ROUTE"
    );
    assert_eq!(
        inherits_off_the_trait,
        Vec::<String>::new(),
        "INHERITS declared outside an impl of BranchView/PagedView/ComputeAction/CommandAction"
    );
}

/// The names bound from a `legacy_actor(` call in `code`: `let x = …legacy_actor(` and
/// `if let Some(x) = …legacy_actor(`, the binding on the same line or the line before.
fn legacy_bindings(code: &str) -> BTreeSet<String> {
    let lines: Vec<&str> = code.lines().collect();
    let mut names = BTreeSet::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("legacy_actor(") || line.contains("fn legacy_actor(") {
            continue;
        }
        let here = if line.contains('=') {
            *line
        } else if i > 0 {
            lines[i - 1]
        } else {
            continue;
        };
        let Some((left, _)) = here.split_once('=') else {
            continue;
        };
        let left = left
            .trim()
            .trim_start_matches("if ")
            .trim_start_matches("let ")
            .trim_start_matches("mut ")
            .trim_start_matches("Some(")
            .trim_end_matches(')')
            .trim();
        if !left.is_empty() && left.chars().all(|c| c.is_alphanumeric() || c == '_') {
            names.insert(left.to_string());
        }
    }
    names
}

/// The argument text of every `Policy::<f>(` call in `code`, with `f`.
fn policy_calls(code: &str) -> Vec<(String, String)> {
    let mut calls = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("Policy::") {
        let after = &rest[at + "Policy::".len()..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let tail = &after[name.len()..];
        if let Some(args) = tail.strip_prefix('(') {
            let mut depth = 1;
            let mut end = 0;
            for (i, c) in args.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            calls.push((name.clone(), args[..end].to_string()));
        }
        rest = &after[name.len()..];
    }
    calls
}

fn mentions(text: &str, name: &str) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == name)
}

#[test]
fn no_result_of_legacy_actor_is_passed_to_policy() {
    // ruling (a) on #767: legacy_actor is a stopgap for downstream business services only
    let mut files = Vec::new();
    rust_files(&api_dir().join("src"), &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in &files {
        let code = code_of(file);
        for line in code.lines() {
            if line.contains("legacy_actor(") && line.contains("Policy::") {
                found.push(format!("{}: {}", relative(file), line.trim()));
            }
        }
        let bound = legacy_bindings(&code);
        for (name, args) in policy_calls(&code) {
            for b in &bound {
                if mentions(&args, b) {
                    found.push(format!("{}: Policy::{name}(…{b}…)", relative(file)));
                }
            }
        }
    }
    assert_eq!(
        found,
        Vec::<String>::new(),
        "a legacy_actor result reaches Policy"
    );
}

#[test]
fn the_v2_rule_is_asked_only_on_the_custody_rows() {
    // #767 §4: `git grep 'Policy::allow(\|Policy::branch(' -- src` lists only the custody path.
    // A call of the v2 rule names `Need::Custody` within the five lines before it.
    let mut files = Vec::new();
    rust_files(&api_dir().join("src"), &mut files);
    files.sort();
    let mut outside = Vec::new();
    for file in &files {
        let code = code_of(file);
        let lines: Vec<&str> = code.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !(line.contains("Policy::allow(") || line.contains("Policy::branch(")) {
                continue;
            }
            let from = i.saturating_sub(5);
            if !lines[from..=i].iter().any(|l| l.contains("Need::Custody")) {
                outside.push(format!("{}: {}", relative(file), line.trim()));
            }
        }
    }
    assert_eq!(
        outside,
        Vec::<String>::new(),
        "the v2 rule asked outside a Need::Custody arm"
    );
}

fn entry_path(name: &str) -> EntryPath {
    match name {
        "Rpc" => EntryPath::Rpc,
        "StaffRpc" => EntryPath::StaffRpc,
        "Upload" => EntryPath::Upload,
        "StaffUpload" => EntryPath::StaffUpload,
        "File" => EntryPath::File,
        "StaffFile" => EntryPath::StaffFile,
        "StaffDesk" => EntryPath::StaffDesk,
        "Devices" => EntryPath::Devices,
        "Kiosk" => EntryPath::Kiosk,
        "OwnerRoute" => EntryPath::OwnerRoute,
        "UserRoute" => EntryPath::UserRoute,
        "Custody" => EntryPath::Custody,
        "PaymentMethods" => EntryPath::PaymentMethods,
        "DrawerPassport" => EntryPath::DrawerPassport,
        "DeviceSetup" => EntryPath::DeviceSetup,
        "StaffBookings" => EntryPath::StaffBookings,
        other => panic!("no entry path {other}"),
    }
}

#[test]
fn no_route_asks_customer_self_while_the_target_is_the_actor() {
    // #767 note on Target.customer_id: Policy::authorize fills the target's customer from the actor,
    // so a CustomerSelf row would always pass. Every path·action the route table is checked with
    // (the 1,425 cases of #766 plus the names the routes ask) has no CustomerSelf row.
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(
            api_dir().join("crates/xc-auth/tests/fixtures/blueprint_policy_cases.json"),
        )
        .expect("the mapped fixture"),
    )
    .expect("JSON");
    let mut asked = BTreeSet::new();
    for case in fixture["cases"].as_array().expect("cases") {
        asked.insert((
            case["path"].as_str().expect("path").to_string(),
            case["action"].as_str().expect("action").to_string(),
        ));
    }
    for (path, action) in [
        ("OwnerRoute", "employees"),
        ("OwnerRoute", "/api/bookings"),
        ("OwnerRoute", "/api/reference-rates"),
        ("UserRoute", "documents"),
        ("Upload", "upload"),
        ("StaffUpload", "upload"),
        ("File", "file"),
        ("StaffFile", "file"),
        ("DrawerPassport", ""),
    ] {
        asked.insert((path.to_string(), action.to_string()));
    }
    assert!(asked.len() > 100, "{} path·action pairs", asked.len());
    let customer_self: Vec<_> = asked
        .iter()
        .filter(|(path, action)| {
            Policy::need(entry_path(path), &Action::new(action.as_str()))
                == Some(Need::Permission(Permission::CustomerSelf))
        })
        .collect();
    assert_eq!(
        customer_self,
        Vec::<&(String, String)>::new(),
        "a CustomerSelf row needs the route to name its customer first (note on #767)"
    );
}

#[test]
fn a_target_that_names_no_customer_never_passes_customer_self() {
    // #767 note on Target.customer_id; design §7 says the target's customer is None unless the
    // route names one. Customer A naming customer B is refused as well.
    let since = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 10, 0, 0, 0).unwrap();
    let customer = Actor::new(
        "user-a",
        "XC",
        vec![ScopedRole::new(Role::Customer, None, since, None)],
        Some("customer-a"),
    );
    let no_one = Target::new("XC", None, None);
    assert_eq!(
        Authorizer::check(Some(&customer), Permission::CustomerSelf, &no_one, now),
        Err(Refusal::OutOfScope),
        "a target naming no customer"
    );
    let somebody_else = Target::new("XC", None, Some("customer-b"));
    assert_eq!(
        Authorizer::check(
            Some(&customer),
            Permission::CustomerSelf,
            &somebody_else,
            now
        ),
        Err(Refusal::OutOfScope),
        "customer A reading customer B"
    );
    let own = Target::new("XC", None, Some("customer-a"));
    assert!(
        Authorizer::check(Some(&customer), Permission::CustomerSelf, &own, now).is_ok(),
        "customer A reading customer A"
    );
}
