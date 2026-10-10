//! Tests for the pure function `api/build.rs` calls to turn the files of
//! `src/presentation/http/v3/commands/` into the generated commands table (§6, §12.20.3).
//!
//! No test here needs a database.

#[path = "../build_support/commands.rs"]
mod commands_gen;

use commands_gen::{CommandTableError, commands_table};

const GOOD: &str = "pub fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n\
                    pub fn export_types(dir: &Path) -> Result<(), ExportError> { Ok(()) }\n";

#[test]
fn every_command_file_is_listed_by_its_module_name_in_order() {
    let table = commands_table(&[
        ("open_shift.rs", GOOD),
        ("close_shift.rs", GOOD),
        ("cancel_order.rs", GOOD),
    ])
    .expect("three well-formed command files");
    let modules: Vec<&str> = table.iter().map(|c| c.module.as_str()).collect();
    assert_eq!(modules, ["cancel_order", "close_shift", "open_shift"]);
    let open = table
        .iter()
        .find(|c| c.module == "open_shift")
        .expect("open_shift");
    assert_eq!(open.file, "open_shift.rs");
}

#[test]
fn a_file_that_is_not_rust_or_is_mod_rs_is_ignored() {
    let table = commands_table(&[
        ("open_shift.rs", GOOD),
        ("mod.rs", GOOD),
        ("README.md", "notes"),
        (".gitkeep", ""),
    ])
    .expect("the non-command files are not commands");
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].module, "open_shift");
}

#[test]
fn a_command_file_without_pub_fn_register_is_refused() {
    let err = commands_table(&[(
        "open_shift.rs",
        "pub fn export_types(dir: &Path) -> Result<(), ExportError> { Ok(()) }\n\
         fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n",
    )])
    .expect_err("no pub fn register");
    assert_eq!(
        err,
        CommandTableError::MissingRegister {
            file: "open_shift.rs".into()
        }
    );
}

#[test]
fn a_command_file_without_pub_fn_export_types_is_refused() {
    let err = commands_table(&[(
        "open_shift.rs",
        "pub fn register(cfg: &mut ServiceConfig, state: &AppState) {}\n",
    )])
    .expect_err("no pub fn export_types");
    assert_eq!(
        err,
        CommandTableError::MissingExportTypes {
            file: "open_shift.rs".into()
        }
    );
}

#[test]
fn a_file_name_that_is_not_a_module_name_is_refused() {
    for name in ["my-view.rs", "View.rs", "1view.rs", "type.rs", ".rs"] {
        let err = commands_table(&[(name, GOOD)]).expect_err("not a command module name");
        assert_eq!(
            err,
            CommandTableError::NotAModuleName { file: name.into() },
            "{name}"
        );
    }
}
