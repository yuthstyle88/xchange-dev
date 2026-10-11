//! #1146 ticket A — the reconcile-check registry is generated from `reconcile/checks/`
//! (design `docs/design/build-time-registries.md` §1.4, §1.6).
//! Written before the implementation by the ticket author (claude/5d45b017). Do not edit, delete or
//! weaken; add tests in a file of your own. No database is needed.

use std::collections::BTreeSet;
use std::path::Path;

use xc_cash::application::reconcile::{CHECK_MODULES, standard_checks};

/// The stems of `src/application/reconcile/checks/*.rs`, without `mod.rs`, sorted.
fn check_files() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/reconcile/checks");
    let mut stems: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs") && n != "mod.rs")
        .map(|n| n.trim_end_matches(".rs").to_owned())
        .collect();
    stems.sort();
    stems
}

#[test]
fn the_generated_set_is_the_files() {
    let generated: Vec<String> = CHECK_MODULES.iter().map(|m| (*m).to_owned()).collect();
    assert_eq!(
        generated,
        check_files(),
        "every reconcile/checks/<name>.rs is in CHECK_MODULES, sorted, through build.rs"
    );
}

#[test]
fn standard_runs_one_check_per_file_with_distinct_ids() {
    let checks = standard_checks();
    assert_eq!(
        checks.len(),
        CHECK_MODULES.len(),
        "standard_checks() has one check per generated module"
    );
    let ids: BTreeSet<u8> = checks.iter().map(|c| c.id()).collect();
    assert_eq!(
        ids.len(),
        checks.len(),
        "two check files return the same check id"
    );
    assert!(
        ids.iter().all(|id| (1..=5).contains(id)),
        "check ids are the numbers of design §6 (1..=5): {ids:?}"
    );
}

#[test]
fn today_s_three_checks_are_registered() {
    let ids: BTreeSet<u8> = standard_checks().iter().map(|c| c.id()).collect();
    for id in [1u8, 2, 5] {
        assert!(
            ids.contains(&id),
            "check {id} is missing from standard_checks(): {ids:?}"
        );
    }
}
