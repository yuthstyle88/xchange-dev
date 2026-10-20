//! #1146 ticket B — the approval-subject registry is generated from
//! `bootstrap/approval_subjects/` (design `docs/design/build-time-registries.md`
//! §2.4, §2.6).
//! Written before the implementation by the ticket author (claude/5d45b017). Do not edit, delete or
//! weaken; add tests in a file of your own. No database is needed: registering stores the pool and
//! never connects.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use xc_approval::application::registry::ApprovalRegistry;
use xchange::bootstrap::approval_subjects::{
    APPROVAL_SUBJECT_MODULES, APPROVAL_SUBJECT_TYPES, APPROVAL_SUBJECTS, RegisterFn,
};
use xchange::presentation::http::v3::commands::make_approval_registry;

/// The stems of `approval_subjects/*.rs`, without `mod.rs`, sorted.
fn subject_files() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/approval_subjects");
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
    let files = subject_files();
    let generated: Vec<String> = APPROVAL_SUBJECT_MODULES
        .iter()
        .map(|m| (*m).to_owned())
        .collect();
    assert_eq!(
        generated, files,
        "every approval_subjects/<name>.rs is in APPROVAL_SUBJECT_MODULES, sorted, through build.rs"
    );
    assert_eq!(
        APPROVAL_SUBJECTS.len(),
        files.len(),
        "one register fn per file"
    );
    assert_eq!(
        APPROVAL_SUBJECT_TYPES.len(),
        files.len(),
        "one TYPE per file"
    );
    let _shape: &[RegisterFn] = APPROVAL_SUBJECTS;
}

#[tokio::test]
async fn every_type_is_registered_once() {
    let types: BTreeSet<&str> = APPROVAL_SUBJECT_TYPES.iter().copied().collect();
    assert_eq!(
        types.len(),
        APPROVAL_SUBJECT_TYPES.len(),
        "two subject files claim the same TYPE; the registry would silently keep only one"
    );
    // Each file's register registers its own TYPE, on a registry of its own.
    for (register, type_name) in APPROVAL_SUBJECTS.iter().zip(APPROVAL_SUBJECT_TYPES) {
        let mut registry = ApprovalRegistry::new();
        register(&mut registry, common::unreachable_pool());
        assert!(
            registry.get(type_name).is_some(),
            "the file whose TYPE is {type_name} does not register {type_name}"
        );
    }
    // And make_approval_registry registers every one of them.
    let registry = make_approval_registry(common::unreachable_pool());
    for type_name in APPROVAL_SUBJECT_TYPES {
        assert!(
            registry.get(type_name).is_some(),
            "make_approval_registry is missing {type_name}"
        );
    }
}

#[tokio::test]
async fn today_s_three_subjects_are_registered() {
    let registry = make_approval_registry(common::unreachable_pool());
    for type_name in [
        "RATE_PUBLICATION",
        "RATE_FEED_PUBLICATION",
        "COUNT_DIFFERENCE",
    ] {
        assert!(
            APPROVAL_SUBJECT_TYPES.contains(&type_name),
            "{type_name} has no file in approval_subjects/: {APPROVAL_SUBJECT_TYPES:?}"
        );
        assert!(
            registry.get(type_name).is_some(),
            "make_approval_registry is missing {type_name}"
        );
    }
}

#[tokio::test]
async fn cash_transfer_is_registered() {
    let registry = make_approval_registry(common::unreachable_pool());
    assert!(
        APPROVAL_SUBJECT_TYPES.contains(&"CASH_TRANSFER"),
        "CASH_TRANSFER has no file in approval_subjects/: {APPROVAL_SUBJECT_TYPES:?}"
    );
    assert!(
        registry.get("CASH_TRANSFER").is_some(),
        "make_approval_registry is missing CASH_TRANSFER"
    );
}
