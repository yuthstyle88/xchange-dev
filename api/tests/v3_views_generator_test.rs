//! Specification tests for V0-B (docs/design/xchange-v3-web.md §5.1 `configure_v3`,
//! §9): the pure function `api/build.rs` calls to turn the files of
//! `src/presentation/http/v3/views/` into the generated view table, so that a
//! view is added by adding its file and no shared file is edited.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! No test here needs a database.

#[path = "../build_support/views.rs"]
mod views_gen;

use views_gen::{ViewTableError, views_table};

const GOOD: &str = "pub fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n\
                    pub fn export_types(dir: &Path) -> Result<(), ExportError> { Ok(()) }\n";

#[test]
fn every_view_file_is_listed_by_its_module_name_in_order() {
    let table = views_table(&[
        ("rates.rs", GOOD),
        ("me.rs", GOOD),
        ("kiosk_queue.rs", GOOD),
    ])
    .expect("three well-formed view files");
    let modules: Vec<&str> = table.iter().map(|v| v.module.as_str()).collect();
    assert_eq!(modules, ["kiosk_queue", "me", "rates"]);
    let me = table.iter().find(|v| v.module == "me").expect("me");
    assert_eq!(me.file, "me.rs");
}

#[test]
fn a_file_that_is_not_rust_is_ignored() {
    let table = views_table(&[("me.rs", GOOD), ("README.md", "notes"), (".gitkeep", "")])
        .expect("the non-Rust files are not views");
    assert_eq!(table.len(), 1);
}

#[test]
fn a_view_file_without_pub_fn_register_is_refused() {
    let err = views_table(&[(
        "rates.rs",
        "pub fn export_types(dir: &Path) -> Result<(), ExportError> { Ok(()) }\n\
         fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n",
    )])
    .expect_err("no pub fn register");
    assert_eq!(
        err,
        ViewTableError::MissingRegister {
            file: "rates.rs".into()
        }
    );
}

#[test]
fn a_view_file_without_pub_fn_export_types_is_refused() {
    let err = views_table(&[(
        "rates.rs",
        "pub fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n",
    )])
    .expect_err("no pub fn export_types");
    assert_eq!(
        err,
        ViewTableError::MissingExportTypes {
            file: "rates.rs".into()
        }
    );
}

#[test]
fn a_file_name_that_is_not_a_module_name_is_refused() {
    for name in ["kiosk-queue.rs", "1rates.rs", "mod.rs", "Rates.rs"] {
        let err = views_table(&[(name, GOOD)]).expect_err("not a view module name");
        assert_eq!(
            err,
            ViewTableError::NotAModuleName { file: name.into() },
            "{name}"
        );
    }
}
