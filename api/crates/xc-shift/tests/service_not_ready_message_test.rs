//! #755: a refused service start says which window it was refused against, and the Bangkok
//! clock that OR-6 left without a caller is gone.
//!
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! What is pinned here (no database; CA-10 #626 retired the machine's start, so the two refused
//! starts moved to `api/tests/service_not_ready_message_test.rs`):
//! - `CustodyTime`'s Bangkok minute-of-day method is named nowhere under `api/`, and the fixed sentence's
//!   `11:00–22:50` appears in no production `.rs` under `api/crates` or `api/src`.
//!
//! The sentence for an open window names the branch's own local hours, so it needs the
//! branch's timezone; it is pinned through the real ports in
//! `api/tests/service_not_ready_message_test.rs`.

use std::path::{Path, PathBuf};

// Re-expected for CA-10 #626 by the ticket author (ruling Q7 A): the machine's `StartService`
// command is retired with the custody command engine; #755's refused-start sentences are pinned
// through `BranchDayService::start_service` (`api/tests/service_not_ready_message_test.rs`).

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("api/")
}

/// Every file under `dir`, build output and dependencies left out.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "target" | "node_modules" | ".git") {
                files_under(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn holding(files: &[PathBuf], needle: &str) -> Vec<String> {
    files
        .iter()
        .filter(|p| {
            std::fs::read(p)
                .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
                .unwrap_or(false)
        })
        .map(|p| p.display().to_string())
        .collect()
}

#[test]
fn the_bangkok_minute_clock_is_gone() {
    // spelt in two halves so this file does not name it
    let needle = ["bangkok_", "minute_of_day"].concat();
    let mut files = Vec::new();
    files_under(&api_dir(), &mut files);
    assert!(files.len() > 100, "api/ was read: {} files", files.len());
    assert_eq!(holding(&files, &needle), Vec::<String>::new());
}

#[test]
fn no_production_code_names_the_old_fixed_window() {
    let needle = ["11:00", "–", "22:50"].concat();
    let api = api_dir();
    let mut files = Vec::new();
    files_under(&api.join("src"), &mut files);
    files_under(&api.join("crates"), &mut files);
    let production: Vec<PathBuf> = files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .filter(|p| !p.components().any(|c| c.as_os_str() == "tests"))
        .collect();
    assert!(
        production.len() > 50,
        "production sources were read: {}",
        production.len()
    );
    assert_eq!(holding(&production, &needle), Vec::<String>::new());
}
