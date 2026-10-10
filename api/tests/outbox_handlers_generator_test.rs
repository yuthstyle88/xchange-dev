//! PL-0a up-front tests (design `docs/design/blueprint/broker-seam.md` §11.1, row "Outbox
//! handlers"): the pure function `api/build.rs` calls to turn the files of
//! `src/bootstrap/outbox_handlers/` into the generated handler list.
//!
//! Written before the implementation by the ticket author. Do not edit, delete or weaken; add
//! tests in a file of your own. No test here needs a database.

#[path = "../build_support/outbox_handlers.rs"]
mod outbox_handlers_gen;

use outbox_handlers_gen::{HandlerTableError, outbox_handlers_table};

const GOOD: &str = "pub fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>> { None }\n";

#[test]
fn every_handler_file_is_listed_by_its_module_name_in_order() {
    let table = outbox_handlers_table(&[
        ("loyalty_earn.rs", GOOD),
        ("broker_attribution.rs", GOOD),
        ("accountzing_gl2.rs", GOOD),
    ])
    .expect("three well-formed handler files");
    let modules: Vec<&str> = table.iter().map(|h| h.module.as_str()).collect();
    assert_eq!(
        modules,
        ["accountzing_gl2", "broker_attribution", "loyalty_earn"]
    );
    let broker = table
        .iter()
        .find(|h| h.module == "broker_attribution")
        .expect("broker_attribution");
    assert_eq!(broker.file, "broker_attribution.rs");
}

#[test]
fn an_empty_directory_gives_an_empty_list() {
    assert_eq!(outbox_handlers_table(&[]), Ok(vec![]));
}

#[test]
fn mod_rs_and_files_that_are_not_rust_are_skipped() {
    let table = outbox_handlers_table(&[
        ("mod.rs", "pub fn registry() {}\n"),
        ("broker_attribution.rs", GOOD),
        ("README.md", "notes"),
        (".gitkeep", ""),
        ("broker_attribution.rs.orig", "junk"),
    ])
    .expect("only one handler file");
    let modules: Vec<&str> = table.iter().map(|h| h.module.as_str()).collect();
    assert_eq!(modules, ["broker_attribution"]);
}

#[test]
fn a_file_without_pub_fn_handler_is_refused() {
    for source in [
        "pub fn other(state: &AppState) -> Option<Arc<dyn OutboxHandler>> { None }\n",
        "fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>> { None }\n",
        "pub(crate) fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>> { None }\n",
        "",
    ] {
        let err = outbox_handlers_table(&[("loyalty_earn.rs", GOOD), ("broker.rs", source)])
            .expect_err("no pub fn handler");
        assert_eq!(
            err,
            HandlerTableError::MissingHandlerFn {
                file: "broker.rs".into()
            },
            "{source:?}"
        );
    }
}

#[test]
fn a_file_name_that_is_not_a_module_name_is_refused() {
    for name in [
        "broker-attribution.rs",
        "1broker.rs",
        "Broker.rs",
        "type.rs",
        ".rs",
    ] {
        let err = outbox_handlers_table(&[(name, GOOD)]).expect_err("not a module name");
        assert_eq!(
            err,
            HandlerTableError::NotAModuleName { file: name.into() },
            "{name}"
        );
    }
}
