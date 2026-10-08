//! Specification tests for V0-D (#508; docs/design/xchange-v3-web.md §12.20.3
//! *Discovery*): compute actions are found the way views are — `api/build.rs`
//! reads `src/presentation/http/v3/actions/` with the same `views_table`, refuses
//! a module name that is both a view and an action (`disjoint`), and generates
//! `ACTIONS` beside `VIEWS`, so an action is added by adding its file.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! No test here needs a database.

#[path = "../build_support/views.rs"]
mod views_gen;

use std::collections::BTreeSet;

use views_gen::{ViewDecl, ViewTableError, disjoint, views_table};
use xchange::presentation::http::v3::{ACTIONS, VIEWS};

const GOOD: &str = "pub fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n\
                    pub fn export_types(dir: &Path) -> Result<(), ExportError> { Ok(()) }\n";

fn decls(files: &[&str]) -> Vec<ViewDecl> {
    let sources: Vec<(&str, &str)> = files.iter().map(|f| (*f, GOOD)).collect();
    views_table(&sources).expect("well-formed files")
}

#[test]
fn an_action_directory_is_read_by_the_same_views_table() {
    let actions = decls(&["shift_valuation.rs", "profit_summary.rs"]);
    let modules: Vec<&str> = actions.iter().map(|a| a.module.as_str()).collect();
    assert_eq!(modules, ["profit_summary", "shift_valuation"]);
}

#[test]
fn views_and_actions_with_different_names_are_disjoint() {
    let views = decls(&["me.rs", "rates.rs", "admin_shift_state.rs"]);
    let actions = decls(&["shift_valuation.rs", "profit_summary.rs"]);
    assert_eq!(disjoint(&views, &actions), Ok(()));
}

#[test]
fn an_empty_or_absent_action_directory_is_disjoint_from_any_views() {
    let views = decls(&["me.rs", "rates.rs"]);
    assert_eq!(disjoint(&views, &[]), Ok(()));
    assert_eq!(disjoint(&[], &[]), Ok(()));
}

#[test]
fn a_module_name_that_is_both_a_view_and_an_action_is_refused() {
    let views = decls(&["me.rs", "shift_valuation.rs", "rates.rs"]);
    let actions = decls(&["profit_summary.rs", "shift_valuation.rs"]);
    assert_eq!(
        disjoint(&views, &actions),
        Err(ViewTableError::NameInBoth {
            module: "shift_valuation".into()
        })
    );
    // and whichever side is passed first
    assert_eq!(
        disjoint(&actions, &views),
        Err(ViewTableError::NameInBoth {
            module: "shift_valuation".into()
        })
    );
}

#[test]
fn a_single_shared_name_is_refused_even_when_it_is_the_only_one() {
    let views = decls(&["me.rs"]);
    let actions = decls(&["me.rs"]);
    assert_eq!(
        disjoint(&views, &actions),
        Err(ViewTableError::NameInBoth {
            module: "me".into()
        })
    );
}

/// The generated table exists beside `VIEWS` (it may be empty until the first
/// action merges) and shares no name with it.
#[test]
fn the_generated_action_table_shares_no_name_with_the_view_table() {
    let views: BTreeSet<&str> = VIEWS.iter().map(|v| v.name).collect();
    let actions: Vec<&str> = ACTIONS.iter().map(|a| a.name).collect();
    let mut sorted = actions.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        actions, sorted,
        "ACTIONS is sorted by module and has no duplicate"
    );
    for name in &actions {
        assert!(!views.contains(name), "{name} is both a view and an action");
    }
}

/// `api/build.rs` reads the actions directory with the same rule, checks the
/// two tables are disjoint, writes `v3_actions.rs`, and reruns when the
/// directory changes (§12.20.3 *Discovery*).
#[test]
fn the_build_script_generates_the_action_table_from_the_actions_directory() {
    let build = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs"))
        .expect("read api/build.rs");
    for needle in [
        "http/v3/actions",
        "disjoint(",
        "v3_actions.rs",
        "ACTIONS",
        "cargo:rerun-if-changed=src/presentation/http/v3/actions",
    ] {
        assert!(build.contains(needle), "api/build.rs has {needle}");
    }
}
