//! Ticket POL (design `docs/design/xchange-v3.md` §5.5.7) — the kiosk's role table has one
//! home, `Policy`. Written before the implementation. Do not edit, delete or weaken.
//!
//! No request can tell this apart: before POL the kiosk route answers a name it does not know
//! itself (`unknown_action_error`, `api/src/presentation/http/kiosk.rs:81-86` at `f72b979`,
//! used at `:213`, `:217`, `:234`) with exactly the sentences `Policy` would give, and asks
//! `KioskMachine::authorize` (`kiosk.rs:237`, `kiosk_machine.rs:25`) — a second copy of the
//! table — for the rest. So what is checked here is where the table is *written*: the sentence
//! only a role check says on this path ("Unknown kiosk action.", R16) is written in `Policy`
//! alone, the machine has no `authorize`, and the route asks `Policy` on the path `Kiosk`.
//! A route that keeps its own check of unknown names, or its own copy of the table, fails here.
//!
//! No database.

use std::fs;
use std::path::{Path, PathBuf};

fn api_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, recursively.
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

/// The source of the server and of every crate of the workspace — not their tests.
fn sources() -> Vec<PathBuf> {
    let api = api_dir();
    let mut files = Vec::new();
    rust_files(&api.join("src"), &mut files);
    for entry in fs::read_dir(api.join("crates")).expect("api/crates") {
        let src = entry.expect("a crate").path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    files.sort();
    files
}

/// The text of a file without its comment lines: a sentence named in a comment is not
/// a sentence said.
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
        .expect("under api/")
        .to_string_lossy()
        .replace('\\', "/")
}

fn files_saying(needle: &str) -> Vec<String> {
    sources()
        .iter()
        .filter(|path| code_of(path).contains(needle))
        .map(|path| relative(path))
        .collect()
}

#[test]
fn the_unknown_kiosk_action_sentence_is_written_in_policy_alone() {
    assert_eq!(
        files_saying("Unknown kiosk action."),
        ["crates/xc-auth/src/domain/policy/value_objects/policy_error.rs"],
        "the route must ask Policy for a name it does not know, not answer it itself"
    );
}

#[test]
fn the_kiosk_machine_keeps_no_role_table() {
    let machine =
        code_of(&api_dir().join("crates/xc-kiosk/src/domain/kiosk/entities/kiosk_machine.rs"));
    assert!(
        !machine.contains("fn authorize"),
        "KioskMachine::authorize is gone: who may call is Policy's"
    );
    assert_eq!(
        files_saying("KioskMachine::authorize"),
        Vec::<String>::new(),
        "nothing asks the machine who may call"
    );
    assert_eq!(
        files_saying("is_desk()"),
        Vec::<String>::new(),
        "nothing outside Policy sorts callers into the desk and the rest"
    );
}

#[test]
fn the_kiosk_route_asks_policy_on_the_kiosk_path() {
    let route = code_of(&api_dir().join("src/presentation/http/kiosk.rs"));
    assert!(
        route.contains("Policy::allow(") && route.contains("EntryPath::Kiosk"),
        "the kiosk route asks Policy::allow on EntryPath::Kiosk"
    );
    assert!(
        !route.contains("fn unknown_action_error"),
        "the route has no answer of its own for a name it does not know"
    );
}
