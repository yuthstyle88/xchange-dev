//! Written before the implementation (#121, item C.3). Do not edit, delete or weaken.
//!
//! The owner reads the differences between v3 and v2 once, in one list, before
//! the first release: `docs/design/xchange-v3-known-differences.md`. Each
//! difference was decided on its own ticket and written down where it was
//! decided — a comment in the source file or test that carries it. This test
//! keeps the list and those comments from drifting apart:
//!
//! 1. every Rust file under `api/` whose text names a decided difference (one
//!    of [`MARKERS`], any letter case) is cited in the list by its path from the
//!    repository root, e.g. `api/src/application/compat/envelope.rs`;
//! 2. every `api/…` path the list cites between backticks exists (a `:line`
//!    or `:a-b` suffix is allowed and ignored).
//!
//! A missing list reads as an empty one, so on `main` without it the first test
//! fails naming every file, not on a read error.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The phrases the source files use for a difference decided on purpose,
/// measured on `main` `3a23eb9`. Compared in lower case.
const MARKERS: [&str; 5] = [
    "known difference",
    "decided difference",
    "deliberate difference",
    "differs from v2 on purpose",
    "v3 differs from v2",
];

/// The list, from the repository root.
const LIST: &str = "docs/design/xchange-v3-known-differences.md";

/// This file names the markers itself; it is not a difference.
const THIS_FILE: &str = "api/tests/known_differences_listed_test.rs";

/// The repository root: the parent of the `api/` package.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("api/ has a parent")
        .to_path_buf()
}

fn read_list() -> String {
    fs::read_to_string(repo_root().join(LIST)).unwrap_or_default()
}

/// Every `.rs` file under `dir`, skipping build output and hidden directories.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            rust_files(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// Paths from the repository root, with `/`, of the files that name a marker.
fn files_with_a_marker() -> BTreeSet<String> {
    let root = repo_root();
    let mut files = Vec::new();
    rust_files(&root.join("api"), &mut files);
    files
        .into_iter()
        .filter_map(|path| {
            let text = fs::read_to_string(&path).ok()?.to_lowercase();
            if !MARKERS.iter().any(|marker| text.contains(marker)) {
                return None;
            }
            let relative = path.strip_prefix(&root).ok()?;
            let relative = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (relative != THIS_FILE).then_some(relative)
        })
        .collect()
}

/// Every backticked `api/…` path the list cites, without a `:line` suffix.
fn cited_paths(list: &str) -> BTreeSet<String> {
    list.split('`')
        .skip(1)
        .step_by(2)
        .filter(|quoted| quoted.starts_with("api/"))
        .map(|quoted| quoted.split(':').next().unwrap_or(quoted).to_string())
        .collect()
}

#[test]
fn the_markers_are_found_where_they_were_measured() {
    // A guard against a scan that finds nothing and so asks for nothing: on
    // `main` `3a23eb9` these three files name a decided difference.
    let found = files_with_a_marker();
    for path in [
        "api/src/application/compat/envelope.rs",
        "api/src/presentation/http/client_address.rs",
        "api/crates/xc-shift/tests/custody_service_test.rs",
    ] {
        assert!(found.contains(path), "{path} is not found by the scan");
    }
    assert!(!found.contains(THIS_FILE));
}

#[test]
fn every_file_that_names_a_decided_difference_is_cited_in_the_list() {
    let list = read_list();
    let missing: Vec<String> = files_with_a_marker()
        .into_iter()
        .filter(|path| !list.contains(path.as_str()))
        .collect();
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "{LIST} does not cite these files, which name a decided difference"
    );
}

#[test]
fn every_path_the_list_cites_exists() {
    let list = read_list();
    assert!(!list.is_empty(), "{LIST} does not exist or is empty");
    let root = repo_root();
    let stale: Vec<String> = cited_paths(&list)
        .into_iter()
        .filter(|path| !root.join(path).exists())
        .collect();
    assert_eq!(
        stale,
        Vec::<String>::new(),
        "{LIST} cites paths that do not exist"
    );
}
