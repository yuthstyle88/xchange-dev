//! Up-front tests for PL-0b #1014 (`docs/design/blueprint/broker-seam.md` §11.1): the pure
//! function `xc-auth/build.rs` calls to turn the files of `src/domain/policy/areas/` into the
//! generated `AREA_ROWS`, so that an area adds its permission rows by adding its own file.
//!
//! Written by the ticket author before the implementation. Do not edit, delete or weaken.
//! No test here needs a database.

#[path = "../build_support/area_rows.rs"]
mod area_rows;

use area_rows::{AreaDecl, AreaTableError, area_rows_table, render};

const GOOD: &str = "use super::AreaRow;\n\
                    pub const ROWS: &[AreaRow] = &[];\n";

#[test]
fn every_area_file_is_listed_by_its_module_name_in_order() {
    let table = area_rows_table(&[
        ("referral.rs", GOOD),
        ("cash_limits.rs", GOOD),
        ("org_admin.rs", GOOD),
    ])
    .expect("three well-formed area files");
    let modules: Vec<&str> = table.iter().map(|d| d.module.as_str()).collect();
    assert_eq!(modules, ["cash_limits", "org_admin", "referral"]);
    let referral = table
        .iter()
        .find(|d| d.module == "referral")
        .expect("referral");
    assert_eq!(referral.file, "referral.rs");
}

#[test]
fn mod_rs_and_files_that_are_not_rust_are_skipped() {
    let table = area_rows_table(&[
        ("mod.rs", "pub struct AreaRow;"),
        ("README.md", "# areas"),
        ("receipts.rs.orig", GOOD),
        ("receipts.rs", GOOD),
    ])
    .expect("one area file among skipped ones");
    let modules: Vec<&str> = table.iter().map(|d| d.module.as_str()).collect();
    assert_eq!(modules, ["receipts"]);
}

#[test]
fn no_area_files_is_an_empty_table() {
    assert_eq!(area_rows_table(&[]).expect("empty"), Vec::<AreaDecl>::new());
    assert_eq!(
        area_rows_table(&[("mod.rs", "")]).expect("mod.rs only"),
        Vec::<AreaDecl>::new()
    );
}

#[test]
fn a_file_name_that_is_not_a_module_name_is_refused() {
    for file in [
        "my-area.rs",
        "Area.rs",
        "1area.rs",
        "type.rs",
        "match.rs",
        ".rs",
    ] {
        assert_eq!(
            area_rows_table(&[(file, GOOD)]),
            Err(AreaTableError::NotAModuleName {
                file: file.to_string()
            }),
            "{file} must be refused"
        );
    }
    // Digits and underscores after the first letter are fine.
    let ok = area_rows_table(&[("app_2_area.rs", GOOD)]).expect("a valid module name");
    assert_eq!(ok[0].module, "app_2_area");
}

#[test]
fn a_file_without_pub_const_rows_is_refused() {
    for source in [
        "",
        "const ROWS: &[AreaRow] = &[];",
        "pub(crate) const ROWS: &[AreaRow] = &[];",
        "pub static ROWS: &[AreaRow] = &[];",
        "pub const ROW: &[AreaRow] = &[];",
        "pub const ROWS: &[Row] = &[];",
    ] {
        assert_eq!(
            area_rows_table(&[("receipts.rs", source)]),
            Err(AreaTableError::MissingRows {
                file: "receipts.rs".to_string()
            }),
            "source {source:?} must be refused"
        );
    }
}

#[test]
fn the_refused_file_is_named_even_among_good_ones() {
    assert_eq!(
        area_rows_table(&[("assistant.rs", GOOD), ("receipts.rs", "fn nothing() {}")]),
        Err(AreaTableError::MissingRows {
            file: "receipts.rs".to_string()
        })
    );
}

#[test]
fn render_declares_each_module_by_path_then_lists_their_rows_in_order() {
    let decls = [
        AreaDecl {
            module: "cash_limits".into(),
            file: "cash_limits.rs".into(),
        },
        AreaDecl {
            module: "referral".into(),
            file: "referral.rs".into(),
        },
    ];
    let out = render(&decls, "/x/areas");
    let compact: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let a = compact
        .find(r#"#[path = "/x/areas/cash_limits.rs"] pub mod cash_limits;"#)
        .expect("cash_limits declared by its path");
    let b = compact
        .find(r#"#[path = "/x/areas/referral.rs"] pub mod referral;"#)
        .expect("referral declared by its path");
    let rows = compact
        .find("pub static AREA_ROWS: &[&[AreaRow]] = &[cash_limits::ROWS, referral::ROWS")
        .expect("AREA_ROWS lists both, in order");
    assert!(
        a < b && b < rows,
        "modules first, in order, then AREA_ROWS:\n{out}"
    );
}

#[test]
fn render_of_no_areas_is_an_empty_area_rows() {
    let out = render(&[], "/x/areas");
    let compact: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        compact.contains("pub static AREA_ROWS: &[&[AreaRow]] = &[];")
            || compact.contains("pub static AREA_ROWS: &[&[AreaRow]] = &[ ];"),
        "{out}"
    );
    assert!(!compact.contains("pub mod"), "{out}");
}
