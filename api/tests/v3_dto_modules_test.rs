//! Specification tests for V0-B §7.7b (ticket #393): the pure function `api/build.rs`
//! calls to turn the files of `src/presentation/dto/v3/` into the generated module
//! list `OUT_DIR/v3_dtos.rs`, so that a view's DTO file is compiled by adding it and
//! `dto/v3/mod.rs` is never edited by a view ticket (behaviour B13).
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! No test here needs a database.

#[path = "../build_support/views.rs"]
mod views_gen;

use views_gen::{ViewTableError, dto_modules};

fn dto_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/dto/v3")
}

#[test]
fn every_dto_file_is_listed_by_its_module_name_in_order() {
    let modules = dto_modules(&[
        ("rates.rs", "pub struct RatesView;"),
        ("me.rs", "pub struct MeView;"),
        ("common.rs", "pub struct Money;"),
        ("kiosk_queue.rs", ""),
    ])
    .expect("four well-formed DTO files");
    assert_eq!(modules, ["common", "kiosk_queue", "me", "rates"]);
}

#[test]
fn mod_rs_and_files_that_are_not_rust_are_ignored() {
    let modules = dto_modules(&[
        (
            "mod.rs",
            "include!(concat!(env!(\"OUT_DIR\"), \"/v3_dtos.rs\"));",
        ),
        ("me.rs", ""),
        ("README.md", "notes"),
        (".gitkeep", ""),
    ])
    .expect("mod.rs and non-Rust files are not DTO modules");
    assert_eq!(modules, ["me"]);
}

#[test]
fn a_file_name_that_is_not_a_module_name_is_refused() {
    for name in ["kiosk-queue.rs", "1rates.rs", "Rates.rs", "fn.rs"] {
        let err = dto_modules(&[("me.rs", ""), (name, "")]).expect_err("not a module name");
        assert_eq!(
            err,
            ViewTableError::NotAModuleName { file: name.into() },
            "{name}"
        );
    }
}

#[test]
fn the_real_dto_directory_lists_common_and_me() {
    let mut files: Vec<(String, String)> = Vec::new();
    for entry in std::fs::read_dir(dto_dir()).expect("read src/presentation/dto/v3") {
        let path = entry.expect("a directory entry").path();
        if path.is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("a UTF-8 file name")
                .to_owned();
            let source = std::fs::read_to_string(&path).unwrap_or_default();
            files.push((name, source));
        }
    }
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let modules = dto_modules(&borrowed).expect("the committed DTO files are well-formed");
    assert!(modules.iter().any(|m| m == "common"), "{modules:?}");
    assert!(modules.iter().any(|m| m == "me"), "{modules:?}");
    assert!(!modules.iter().any(|m| m == "mod"), "{modules:?}");
}

#[test]
fn dto_v3_mod_rs_only_includes_the_generated_list() {
    let source = std::fs::read_to_string(dto_dir().join("mod.rs")).expect("dto/v3/mod.rs");
    assert!(
        source.contains(r#"include!(concat!(env!("OUT_DIR"), "/v3_dtos.rs"));"#),
        "dto/v3/mod.rs includes OUT_DIR/v3_dtos.rs: {source}"
    );
    for line in source.lines() {
        let code = line.trim_start();
        assert!(
            !(code.starts_with("pub mod ") || code.starts_with("mod ")),
            "dto/v3/mod.rs has a hand-written module line {line:?}; the list is generated (§7.7b)"
        );
    }
}
