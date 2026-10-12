//! XS-0 #1375 up-front tests (design `docs/design/blueprint/membership-points.md` §6.2 "Registration", §9.3 row
//! XS-0; `broker-seam.md` §11.1): the pure function `api/build.rs` calls to turn the files of
//! `src/bootstrap/exchange_effects/` into the generated effect list — the same mechanism as PL-0a's
//! `outbox_handlers` (lane controller claude/ad05bf8d's ruling on #1375).
//!
//! Written before the implementation by the ticket author (claude/29df2ec5). Do not edit, delete or weaken; add
//! tests in a file of your own. No test here needs a database.

#[path = "../build_support/exchange_effects.rs"]
mod exchange_effects_gen;

use exchange_effects_gen::{EffectTableError, exchange_effects_table};

const GOOD: &str = "pub fn effect(state: &AppState) -> Option<Arc<dyn ExchangeEffect>> { None }\n";

/// A destination is one file: it is listed under its module name, and the list is in file-name order —
/// the order the effects of one phase run in.
#[test]
fn every_effect_file_is_listed_by_its_module_name_in_file_name_order() {
    let table = exchange_effects_table(&[
        ("marketing.rs", GOOD),
        ("loyalty.rs", GOOD),
        ("a_first.rs", GOOD),
    ])
    .expect("three well-formed effect files");
    let modules: Vec<&str> = table.iter().map(|e| e.module.as_str()).collect();
    assert_eq!(modules, ["a_first", "loyalty", "marketing"]);
    let loyalty = table
        .iter()
        .find(|e| e.module == "loyalty")
        .expect("loyalty");
    assert_eq!(loyalty.file, "loyalty.rs");
}

#[test]
fn a_directory_with_only_mod_rs_gives_an_empty_list() {
    assert_eq!(exchange_effects_table(&[]), Ok(vec![]));
    assert_eq!(
        exchange_effects_table(&[("mod.rs", "pub fn effects() {}\n")]),
        Ok(vec![])
    );
}

#[test]
fn mod_rs_and_files_that_are_not_rust_are_skipped() {
    let table = exchange_effects_table(&[
        ("mod.rs", "pub fn effect(state: &AppState) {}\n"),
        ("loyalty.rs", GOOD),
        ("README.md", "notes"),
        (".gitkeep", ""),
        ("loyalty.rs.orig", "junk"),
    ])
    .expect("only one effect file");
    let modules: Vec<&str> = table.iter().map(|e| e.module.as_str()).collect();
    assert_eq!(modules, ["loyalty"]);
}

#[test]
fn a_file_without_pub_fn_effect_is_refused_and_named() {
    for source in [
        "pub fn other(state: &AppState) -> Option<Arc<dyn ExchangeEffect>> { None }\n",
        "fn effect(state: &AppState) -> Option<Arc<dyn ExchangeEffect>> { None }\n",
        "pub(crate) fn effect(state: &AppState) -> Option<Arc<dyn ExchangeEffect>> { None }\n",
        "pub fn effects(state: &AppState) -> ExchangeEffects { todo!() }\n",
        "",
    ] {
        let err = exchange_effects_table(&[("loyalty.rs", GOOD), ("marketing.rs", source)])
            .expect_err("no pub fn effect");
        assert_eq!(
            err,
            EffectTableError::MissingEffectFn {
                file: "marketing.rs".into()
            },
            "{source:?}"
        );
    }
}

#[test]
fn a_file_name_that_is_not_a_module_name_is_refused_and_named() {
    for name in [
        "loyalty-earn.rs",
        "1loyalty.rs",
        "Loyalty.rs",
        "type.rs",
        ".rs",
    ] {
        let err = exchange_effects_table(&[(name, GOOD)]).expect_err("not a module name");
        assert_eq!(
            err,
            EffectTableError::NotAModuleName { file: name.into() },
            "{name}"
        );
    }
}
