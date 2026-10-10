//! #755: a refused service start says which window it was refused against, and the Bangkok
//! clock that OR-6 left without a caller is gone.
//!
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! What is pinned here (the machine, no database):
//! - a branch whose hours are not set (`ServiceWindow::NotSet`) is refused with
//!   `สาขานี้ยังไม่ได้ตั้งเวลาทำการ ให้ผู้ดูแลตั้งเวลาก่อนเปิดบริการ`;
//! - a branch closed today (`ServiceWindow::Closed`) is refused with `วันนี้สาขาปิด`;
//! - both are still `ServiceNotReady` (matched with `{ .. }`, so the variant may carry the
//!   window);
//! - `CustodyTime`'s Bangkok minute-of-day method is named nowhere under `api/`, and the fixed sentence's
//!   `11:00–22:50` appears in no production `.rs` under `api/crates` or `api/src`.
//!
//! The sentence for an open window names the branch's own local hours, so it needs the
//! branch's timezone; it is pinned through the real ports in
//! `api/tests/service_not_ready_message_test.rs`.

mod custody_common;

use std::path::{Path, PathBuf};

use custody_common::*;

use CustodyError as E;

const NOT_SET: &str = "สาขานี้ยังไม่ได้ตั้งเวลาทำการ ให้ผู้ดูแลตั้งเวลาก่อนเปิดบริการ";
const CLOSED_TODAY: &str = "วันนี้สาขาปิด";

fn start(d: &Desk) -> CustodyCommand {
    CustodyCommand::StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: d.rates_updated.clone(),
    }
}

/// BE1 starts the service of a ready desk at noon in Bangkok, told `window`.
fn refused_start(window: ServiceWindow) -> CustodyError {
    let d = Desk::ready();
    let facts = CustodyFacts {
        service_window: window,
        ..d.facts()
    };
    match CustodyMachine::apply(
        &d.state,
        &start(&d),
        &actor("BE1"),
        &facts,
        CustodyTime::from_epoch_millis(NOON),
    ) {
        Err(e) => e,
        Ok(t) => panic!("the start is refused, got phase {:?}", t.state.phase),
    }
}

#[test]
fn a_branch_without_hours_is_told_to_have_them_set() {
    let e = refused_start(ServiceWindow::NotSet);
    assert!(matches!(e, E::ServiceNotReady { .. }), "{e}");
    assert_eq!(e.to_string(), NOT_SET);
}

#[test]
fn a_branch_closed_today_is_told_so() {
    let e = refused_start(ServiceWindow::Closed);
    assert!(matches!(e, E::ServiceNotReady { .. }), "{e}");
    assert_eq!(e.to_string(), CLOSED_TODAY);
}

// ── source guard ─────────────────────────────────────────────────────────────

/// `api/`, from this crate's manifest (`api/crates/xc-shift`).
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
