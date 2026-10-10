//! APP-3 (#993): the template registry (design `customer-app.md` P10, §5: "adding a template is
//! adding one file … two templates with one key fail the build"). Written before the
//! implementation by the test author (claude/dell-claude-2). Do not edit, delete or weaken; add
//! tests in a file of your own. No test here needs a database.

mod common;

#[path = "../build_support/template_table.rs"]
#[allow(dead_code)]
mod template_table;

use common::*;
use template_table::{TemplateDecl, TemplateTableError, render, template_table};

fn template_file(key: &str) -> String {
    format!("pub const KEY: &str = \"{key}\";\npub struct T;\n")
}

#[test]
fn the_registry_finds_a_template_by_its_key() {
    let registry = registry();
    let found = registry.get(TRANSACTIONAL).expect("a registered key");
    assert_eq!(found.key(), TRANSACTIONAL);
    assert_eq!(found.version(), TRANSACTIONAL_VERSION);
    assert_eq!(found.kind(), TemplateKind::Transactional);
    assert_eq!(
        registry.get(MARKETING).map(|t| t.kind()),
        Some(TemplateKind::Marketing)
    );
    assert!(registry.get("no_such_template").is_none());
}

#[test]
fn two_templates_with_one_key_are_refused_by_the_registry() {
    let result = TemplateRegistry::from_templates(vec![
        Box::new(TestTransactional),
        Box::new(TestMarketing),
        Box::new(TestTransactional),
    ]);
    match result {
        Err(e) => assert_eq!(e, DuplicateTemplateKey(TRANSACTIONAL.into())),
        Ok(_) => panic!("a duplicate key must be refused"),
    }
}

#[test]
fn the_generated_registry_builds() {
    // The templates of src/templates/ (none until APP-10 adds queue_called.rs): building it must
    // not panic, which is the generated list's own duplicate check passing.
    let _ = TemplateRegistry::generated();
}

#[test]
fn every_template_file_is_listed_by_module_name_in_file_name_order_with_its_key() {
    let a = template_file("queue_called");
    let b = template_file("booking_reminder");
    let table = template_table(&[("queue_called.rs", &a), ("booking_reminder.rs", &b)])
        .expect("two well-formed template files");
    assert_eq!(
        table,
        vec![
            TemplateDecl {
                module: "booking_reminder".into(),
                file: "booking_reminder.rs".into(),
                key: "booking_reminder".into(),
            },
            TemplateDecl {
                module: "queue_called".into(),
                file: "queue_called.rs".into(),
                key: "queue_called".into(),
            },
        ]
    );
}

#[test]
fn mod_rs_and_files_that_are_not_rust_are_skipped() {
    let a = template_file("queue_called");
    let table = template_table(&[
        ("mod.rs", "include!(\"x\");"),
        ("README.md", "# templates"),
        ("queue_called.rs.orig", &a),
        ("queue_called.rs", &a),
    ])
    .expect("one template file");
    let modules: Vec<&str> = table.iter().map(|d| d.module.as_str()).collect();
    assert_eq!(modules, ["queue_called"]);
}

#[test]
fn two_template_files_with_one_key_fail_the_build() {
    let a = template_file("queue_called");
    let b = template_file("queue_called");
    let c = template_file("rate_alert");
    let err = template_table(&[
        ("queue_called.rs", &a),
        ("rate_alert.rs", &c),
        ("queue_called_v2.rs", &b),
    ])
    .expect_err("one key in two files");
    assert_eq!(
        err,
        TemplateTableError::DuplicateKey {
            key: "queue_called".into(),
            files: vec!["queue_called.rs".into(), "queue_called_v2.rs".into()],
        }
    );
}

#[test]
fn a_template_file_without_a_key_fails_the_build() {
    let err = template_table(&[("queue_called.rs", "pub struct T;\n")]).expect_err("no KEY");
    assert_eq!(
        err,
        TemplateTableError::MissingKey {
            file: "queue_called.rs".into()
        }
    );
}

#[test]
fn a_file_whose_name_is_not_a_module_name_fails_the_build() {
    let a = template_file("queue_called");
    let err = template_table(&[("queue-called.rs", &a)]).expect_err("a dash");
    assert_eq!(
        err,
        TemplateTableError::NotAModuleName {
            file: "queue-called.rs".into()
        }
    );
}

#[test]
fn render_declares_each_template_module_by_path_and_the_generated_function() {
    let decls = vec![TemplateDecl {
        module: "queue_called".into(),
        file: "queue_called.rs".into(),
        key: "queue_called".into(),
    }];
    let code = render(&decls, "/src/templates");
    assert!(
        code.contains("#[path = \"/src/templates/queue_called.rs\"]"),
        "{code}"
    );
    assert!(code.contains("pub mod queue_called;"), "{code}");
    assert!(code.contains("pub fn generated()"), "{code}");
}
