//! RBAC C1b (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.5, pos108's
//! `permission_catalog_test` idea, gap G3): every v3 route's `INHERITS (EntryPath, action)`
//! resolves to a `Need`. `or4_routes_guard_test.rs` checks only that a route declares one; an
//! action with no row fails closed only at run time.
//! Written by the ticket author (claude/83f682) as part of C1b's test commit. Do not edit, delete
//! or weaken; add tests in a file of your own. No database.

use std::fs;
use std::path::{Path, PathBuf};

use xc_auth::domain::policy::entities::policy::Policy;
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

/// The files `build.rs` registers as v3 routes: every `.rs` under `views/`, `actions/` and
/// `commands/` but a `mod.rs` (the same list as `or4_routes_guard_test.rs`).
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

/// Every `EntryPath`. `exhaustive` fails to compile when a variant is added and not listed here.
const ALL: [EntryPath; 16] = [
    EntryPath::Rpc,
    EntryPath::StaffRpc,
    EntryPath::Upload,
    EntryPath::StaffUpload,
    EntryPath::File,
    EntryPath::StaffFile,
    EntryPath::StaffDesk,
    EntryPath::Devices,
    EntryPath::Kiosk,
    EntryPath::OwnerRoute,
    EntryPath::UserRoute,
    EntryPath::Custody,
    EntryPath::PaymentMethods,
    EntryPath::DrawerPassport,
    EntryPath::DeviceSetup,
    EntryPath::StaffBookings,
];

#[allow(dead_code)]
fn exhaustive(p: EntryPath) {
    match p {
        EntryPath::Rpc
        | EntryPath::StaffRpc
        | EntryPath::Upload
        | EntryPath::StaffUpload
        | EntryPath::File
        | EntryPath::StaffFile
        | EntryPath::StaffDesk
        | EntryPath::Devices
        | EntryPath::Kiosk
        | EntryPath::OwnerRoute
        | EntryPath::UserRoute
        | EntryPath::Custody
        | EntryPath::PaymentMethods
        | EntryPath::DrawerPassport
        | EntryPath::DeviceSetup
        | EntryPath::StaffBookings => {}
    }
}

/// `EntryPath::<Variant>` by its name; `None` for a name that is no variant.
fn entry_path(name: &str) -> Option<EntryPath> {
    ALL.into_iter().find(|p| format!("{p:?}") == name)
}

/// The text of `code` without comment lines, on one line, whitespace runs folded to one space.
fn flat(code: &str) -> String {
    code.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The string literal right after `prefix` in `text` (`prefix "value"`), if there is one.
fn literal_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let at = text.find(prefix)? + prefix.len();
    let rest = text[at..].trim_start().strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// The value of `const <name>: &str = "…"` (or `&'static str`) in `text`; for `Self::NAME`, the
/// nearest `const NAME` before `before` (the impl the `INHERITS` is in).
fn const_value(text: &str, name: &str, before: usize) -> Option<String> {
    let forms = |n: &str| {
        [
            format!("const {n}: &'static str ="),
            format!("const {n}: &str ="),
        ]
    };
    if name == "Self::NAME" {
        let head = &text[..before];
        let at = forms("NAME")
            .iter()
            .filter_map(|f| head.rfind(f.as_str()))
            .max()?;
        return forms("NAME")
            .iter()
            .find_map(|f| literal_after(&head[at..], f))
            .map(str::to_owned);
    }
    forms(name)
        .iter()
        .find_map(|f| literal_after(text, f))
        .map(str::to_owned)
}

/// Every `const INHERITS … = (EntryPath::X, <name>)` in `code`, as `(X, name)`; `<name>` is a
/// string literal or a constant of the same file. Skipped: the trait's own declaration (no value),
/// `= Other::INHERITS` (forwards one read where it is) and a macro parameter (`$name`). `Err`
/// names any other declaration this reader cannot resolve, so a new form is noticed.
fn inherits_of(code: &str) -> Vec<Result<(String, String), String>> {
    let text = flat(code);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("const INHERITS") {
        let start = from + found;
        from = start + "const INHERITS".len();
        let tail = &text[from..];
        let (Some(eq), semi) = (tail.find('='), tail.find(';')) else {
            continue;
        };
        if semi.is_some_and(|s| s < eq) {
            continue; // the trait's declaration, no value
        }
        let value = tail[eq + 1..].trim_start();
        if !value.starts_with('(') {
            continue; // `= Other::INHERITS`: forwards a declaration this reader checks where it is
        }
        let Some(close) = value.find(')') else {
            continue;
        };
        let inner = value[..close].trim_start_matches('(');
        let Some((path, name)) = inner.split_once(',') else {
            out.push(Err(format!("cannot read `{}`", &value[..close + 1])));
            continue;
        };
        let variant = path.trim().trim_start_matches("EntryPath::").to_owned();
        let name = name.trim();
        if name.starts_with('$') {
            continue; // a macro parameter: the invocations are not read (queue.rs)
        }
        let resolved = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
            Some(lit) => Some(lit.to_owned()),
            None => const_value(&text, name, start),
        };
        match resolved {
            Some(n) => out.push(Ok((variant, n))),
            None => out.push(Err(format!("cannot resolve `{name}`"))),
        }
    }
    out
}

#[test]
fn the_reader_finds_a_known_declaration() {
    let code = "trait T { const INHERITS: (EntryPath, &'static str); }\n\
        impl CommandAction for X {\n    const INHERITS: (EntryPath, &'static str) =\n        (EntryPath::Rpc, \"userGrant\");\n}\n\
        const VIEW_NAME: &str = \"adminUsers\";\n\
        impl A for Y { const NAME: &'static str = \"branchDayStart\"; const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, Self::NAME); }\n\
        impl A for Z { const NAME: &'static str = \"branchDayCutoff\"; const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, Self::NAME); }\n\
        impl B for W { const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, VIEW_NAME); }";
    let pair = |p: &str, n: &str| Ok((p.to_owned(), n.to_owned()));
    assert_eq!(
        inherits_of(code),
        vec![
            pair("Rpc", "userGrant"),
            pair("Rpc", "branchDayStart"),
            pair("Kiosk", "branchDayCutoff"),
            pair("Rpc", "adminUsers"),
        ]
    );
}

#[test]
fn every_v3_route_inherits_resolves_to_a_need() {
    let files = v3_route_files();
    let mut found = 0;
    let mut unresolved = Vec::new();
    for file in &files {
        let code =
            fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        let rel = file
            .strip_prefix(api_dir())
            .unwrap_or(file)
            .display()
            .to_string();
        for declared in inherits_of(&code) {
            found += 1;
            let (variant, action) = match declared {
                Ok(pair) => pair,
                Err(why) => {
                    unresolved.push(format!("{rel}: {why}"));
                    continue;
                }
            };
            match entry_path(&variant) {
                None => unresolved.push(format!("{rel}: unknown EntryPath::{variant}")),
                Some(path) => {
                    if Policy::need(path, &Action::new(action.clone())).is_none() {
                        unresolved.push(format!("{rel}: ({variant}, \"{action}\") has no row"));
                    }
                }
            }
        }
    }
    assert!(found >= 40, "INHERITS declarations found: {found}");
    assert_eq!(
        unresolved,
        Vec::<String>::new(),
        "INHERITS that resolve to no Need"
    );
}
