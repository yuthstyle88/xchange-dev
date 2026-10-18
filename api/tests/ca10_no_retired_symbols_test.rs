//! CA-10 (#626): the request model, the custodian, M2's marker and `xc-exchange`'s customer shims are gone.
//! Design `docs/design/blueprint/cash-shift-transfer.md` (#582) §6 (what is retired), §9.1 row CA-10 ("`grep` finds
//! no `opsPropose`/`custody_requests`/`custodian_id`/`opened_by_count` outside migrations"); #554 §9.2 and the
//! lane controller's ruling (a) of 2026-10-09 on #626 (the shims and the `Mode` re-export are deleted, their
//! importers point at `xc-customer` / `xc_kernel::mode::Mode`); the ruling on CA-10a #1632 (option B, 2026-10-13:
//! the `stopgap (#626)` line in `api/tests/shift_setup_common/mod.rs` goes with CA-10).
//!
//! Written before the implementation by the ticket author (claude/c4d72c, for the BE lane controller claude/7ca80c).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Scope of the name scan: production sources only (`api/src` and every `api/crates/*/src`). Tests are left out on
//! purpose: CA-10's own M3 test (`ca10_m3_migration_test.rs`) must name the dropped tables and columns to check
//! that they are gone. Migrations are SQL, so they are outside a `.rs` scan anyway.

use std::path::{Path, PathBuf};

/// §9.1 row CA-10, word for word.
const RETIRED_NAMES: [&str; 4] = [
    "opsPropose",
    "custody_requests",
    "custodian_id",
    "opened_by_count",
];

/// The shim modules of `xc-exchange` (#554 §9.2).
const SHIM_DIRS: [&str; 3] = [
    "crates/xc-exchange/src/domain/customer",
    "crates/xc-exchange/src/application/customer",
    "crates/xc-exchange/src/infrastructure/customer",
];

/// The stopgap the ruling on #1632 sends to CA-10.
const STOPGAP: &str = "stopgap (#626)";

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "target" || name == "node_modules" || name.starts_with('.') {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Production sources: `api/src` and `api/crates/<crate>/src`.
fn production_files() -> Vec<PathBuf> {
    let api = api_dir();
    let mut files = Vec::new();
    rust_files(&api.join("src"), &mut files);
    let crates = std::fs::read_dir(api.join("crates")).expect("read api/crates");
    for krate in crates {
        let krate = krate.expect("crate entry").path();
        rust_files(&krate.join("src"), &mut files);
    }
    files.sort();
    files
}

/// Every `.rs` file under `api/`, this guard excepted.
fn every_api_file() -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&api_dir(), &mut files);
    files.retain(|f| !f.ends_with("tests/ca10_no_retired_symbols_test.rs"));
    files.sort();
    files
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `word` appears in `line` as a whole identifier (not as part of a longer one).
fn has_word(line: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(i) = line[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        let before = line[..start].chars().next_back();
        let after = line[end..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return true;
        }
        from = end;
    }
    false
}

/// A path through one of `xc-exchange`'s customer shims: `xc_exchange::<layer>::customer` from outside the crate,
/// or `crate::<layer>::customer` inside it.
fn names_a_shim(line: &str, file_is_in_xc_exchange: bool) -> bool {
    ["domain", "application", "infrastructure"]
        .iter()
        .any(|layer| {
            has_word(line, &format!("xc_exchange::{layer}::customer"))
                || (file_is_in_xc_exchange && has_word(line, &format!("crate::{layer}::customer")))
        })
}

fn hits<F: Fn(&Path, &str) -> bool>(files: &[PathBuf], hit: F) -> Vec<String> {
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if hit(file, line) {
                out.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    out
}

#[test]
fn no_production_source_names_the_request_model_the_custodian_or_m2s_marker() {
    let found = hits(&production_files(), |_, line| {
        RETIRED_NAMES.iter().any(|w| has_word(line, w))
    });
    assert!(
        found.is_empty(),
        "CA-10 retires {RETIRED_NAMES:?} (design §6, §9.1). {} site(s) left in api/src or api/crates/*/src:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn xc_exchange_no_longer_has_customer_shim_modules() {
    let api = api_dir();
    let left: Vec<&str> = SHIM_DIRS
        .iter()
        .copied()
        .filter(|d| api.join(d).exists())
        .collect();
    assert!(
        left.is_empty(),
        "CA-10 deletes xc-exchange's customer shims and the Mode re-export (#554 §9.2); still present: {left:?}"
    );
}

#[test]
fn nothing_under_api_imports_through_the_customer_shims() {
    let found = hits(&every_api_file(), |file, line| {
        let path = file.to_string_lossy();
        if path.contains("/crates/xc-customer/") {
            return false;
        }
        names_a_shim(line, path.contains("/crates/xc-exchange/"))
    });
    assert!(
        found.is_empty(),
        "point these at xc-customer / xc_kernel::mode::Mode (ruling (a) on #626). {} site(s):\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn the_stopgap_for_626_is_gone() {
    let found = hits(&every_api_file(), |_, line| line.contains(STOPGAP));
    assert!(
        found.is_empty(),
        "the ruling on #1632 (option B) sends this stopgap to CA-10: change ShiftRepository::active / active_in \
         so a count-opened shift is the active shift, and remove the stopgap. {} site(s):\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn the_guard_is_not_blind() {
    // Whole identifiers are hits, however they are written in code.
    for line in [
        "let id = state.custodian_id;",
        "\"SELECT * FROM custody_requests WHERE branch_id = $1\"",
        "\"opsPropose\" => propose(ctx),",
        "AND opened_by_count IS NULL",
    ] {
        assert!(RETIRED_NAMES.iter().any(|w| has_word(line, w)), "{line}");
    }
    // Longer identifiers that only contain a retired name are not hits.
    for line in [
        "let custodian_ids = 1;",
        "fn opened_by_count_of() {}",
        "x_custody_requests_y",
    ] {
        assert!(!RETIRED_NAMES.iter().any(|w| has_word(line, w)), "{line}");
    }
    // Shim paths, from outside and from inside xc-exchange.
    assert!(names_a_shim(
        "use xc_exchange::domain::customer::value_objects::mode::Mode;",
        false
    ));
    assert!(names_a_shim(
        "use xc_exchange::infrastructure::customer::pg::PgCustomers;",
        false
    ));
    assert!(names_a_shim(
        "use crate::application::customer::service;",
        true
    ));
    // `crate::…::customer` is a shim only inside xc-exchange; a longer module name is not a hit.
    assert!(!names_a_shim(
        "use crate::application::customer::service;",
        false
    ));
    assert!(!names_a_shim(
        "use crate::application::customer_signup::Signup;",
        true
    ));
    assert!(!names_a_shim(
        "use xc_customer::domain::customer::Customer;",
        false
    ));
}
