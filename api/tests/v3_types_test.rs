//! Specification tests for V0-B (docs/design/xchange-v3-web.md §5.1 "Types shared
//! with the web", §9): the TypeScript types the screens use are generated from
//! the Rust DTOs by ts-rs, committed under `web/src/shared/api/generated/`, and
//! cannot drift from them.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `presentation::dto::v3::common::export_types(dir)` writes the shared types
//!   (`ViewAnswer<T>`, the refusal body, `Page<T>`, ids, `Money`, `Rate`, `Me`)
//!   to `dir/common.ts`.
//! - `presentation::http::v3::VIEWS` is the generated table of the views in
//!   `http/v3/views/`; each entry's `export_types(dir)` writes that view's DTOs
//!   to `dir/<name>.ts` (and, through ts-rs, the common types they use).
//! - `presentation::http::v3::ACTIONS` is the generated table of the compute
//!   actions in `http/v3/actions/` (V0-D, #508, §12.20.3 *Types*); each entry
//!   writes its DTOs the same way, under the same rules. It may be empty until
//!   the first action merges. (The controller's amendment for #508.)
//!
//! No test here needs a database.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use xchange::presentation::dto::v3::common;
use xchange::presentation::http::v3::{ACTIONS, VIEWS};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A new empty directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("xc-v3-types-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/src/shared/api/generated")
}

fn ts_files(dir: &Path) -> BTreeSet<String> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".ts"))
            .collect(),
        Err(_) => BTreeSet::new(),
    }
}

fn export_everything(dir: &Path) {
    common::export_types(dir).expect("export the common types");
    for view in VIEWS {
        (view.export_types)(dir).unwrap_or_else(|e| panic!("export view {}: {e}", view.name));
    }
    for action in ACTIONS {
        (action.export_types)(dir).unwrap_or_else(|e| panic!("export action {}: {e}", action.name));
    }
}

/// `XC_UPDATE_GENERATED=1 cargo test --test v3_types_test` rewrites the
/// committed files from the DTOs; CI never sets it, so there it only compares.
fn update_committed_if_asked() {
    if std::env::var("XC_UPDATE_GENERATED").as_deref() != Ok("1") {
        return;
    }
    let dir = committed_dir();
    std::fs::create_dir_all(&dir).expect("create web/src/shared/api/generated");
    for stale in ts_files(&dir) {
        std::fs::remove_file(dir.join(stale)).expect("remove a stale generated file");
    }
    export_everything(&dir);
}

#[test]
fn the_committed_types_are_what_the_rust_dtos_generate() {
    update_committed_if_asked();
    let scratch = Scratch::new();
    export_everything(&scratch.0);
    let produced = ts_files(&scratch.0);
    assert!(produced.contains("common.ts"), "produced {produced:?}");
    let committed = ts_files(&committed_dir());
    assert_eq!(
        committed, produced,
        "the files in web/src/shared/api/generated/ are exactly the generated ones \
         (run `cargo test` in api/ and commit what it writes)"
    );
    for file in &produced {
        let want = std::fs::read_to_string(scratch.0.join(file)).expect("read produced");
        let have = std::fs::read_to_string(committed_dir().join(file)).unwrap_or_default();
        assert_eq!(
            have, want,
            "web/src/shared/api/generated/{file} differs from what the Rust DTOs generate"
        );
    }
}

#[test]
fn each_view_writes_only_its_own_file_and_common() {
    assert!(!VIEWS.is_empty(), "the view table is empty");
    for view in VIEWS.iter().chain(ACTIONS) {
        let scratch = Scratch::new();
        (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {}: {e}", view.name));
        let files = ts_files(&scratch.0);
        let own = format!("{}.ts", view.name);
        assert!(
            files.contains(&own),
            "{}: writes {own}; wrote {files:?}",
            view.name
        );
        for f in &files {
            assert!(
                *f == own || f == "common.ts" || f == "transaction_row.ts",
                "{}: writes {f}, which is neither {own}, common.ts nor transaction_row.ts",
                view.name
            );
        }
    }
}

#[test]
fn the_me_view_is_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(names.contains(&"me"), "views {names:?}");
}

#[test]
fn versions_are_numbers_and_money_is_a_decimal_string_on_the_wire() {
    let scratch = Scratch::new();
    export_everything(&scratch.0);
    let common_ts = std::fs::read_to_string(scratch.0.join("common.ts")).expect("common.ts");
    assert!(
        common_ts.contains("export type Money = string;"),
        "Money is a decimal string (W7): {common_ts}"
    );
    assert!(
        common_ts.contains("export type Rate = string;"),
        "Rate is a decimal string (W7): {common_ts}"
    );
    let answer = common_ts
        .lines()
        .find(|l| l.starts_with("export type ViewAnswer<"))
        .unwrap_or_else(|| panic!("ViewAnswer in common.ts: {common_ts}"));
    assert!(answer.contains("version: number"), "{answer}");
    assert!(answer.contains("changed: boolean"), "{answer}");
    assert!(answer.contains("data?: T"), "{answer}");
    for file in ts_files(&scratch.0) {
        let text = std::fs::read_to_string(scratch.0.join(&file)).expect("read");
        assert!(
            !text.contains("bigint"),
            "{file}: JSON carries numbers, never bigint: {text}"
        );
    }
}
