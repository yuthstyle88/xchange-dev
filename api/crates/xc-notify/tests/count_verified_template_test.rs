//! #5643: the `count_verified` notification template, sent to the person whose cash count was verified.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-16). Do not edit, delete or
//! weaken these tests; add your own in another file. No test here needs a database.
//! Design: `docs/design/blueprint/cash-count-notices.md`.
//!
//! The template is one new file, `src/templates/count_verified.rs` (`KEY = "count_verified"`, transactional).
//! Its params (set by `xc-cash`'s `verify_in`, pinned in `xc-cash/tests/count_verify_notice_test.rs`):
//! `kind` (`OPEN`, `CLOSE`, `SAFE_OPEN`, `SAFE`), `branch`, `counted_on` (`YYYY-MM-DD`), `counted_time` (`HH:MM`).
//!
//! Server texts are Thai only in feature work (`docs/design/walkin-queue.md:15`), so every locale renders the same
//! Thai title and body. The kind is named as `count_rejected` names it.

mod common;

use common::*;

const TITLE: &str = "ยอดนับเงินได้รับการรับรองแล้ว";

fn verified(kind: &str) -> std::collections::BTreeMap<String, String> {
    params(&[
        ("kind", kind),
        ("branch", "MAIN-01"),
        ("counted_on", "2026-10-30"),
        ("counted_time", "10:52"),
    ])
}

fn render(locale: Locale, kind: &str) -> Rendered {
    let registry = TemplateRegistry::generated();
    let template = registry
        .get("count_verified")
        .expect("the count_verified template");
    template.render(locale, &verified(kind))
}

#[test]
fn the_template_is_registered_and_transactional() {
    let registry = TemplateRegistry::generated();
    let template = registry
        .get("count_verified")
        .expect("the count_verified template");
    assert_eq!(template.key(), "count_verified");
    assert_eq!(template.kind(), TemplateKind::Transactional);
}

#[test]
fn t1_the_thai_text_names_the_kind_time_and_branch() {
    let r = render(Locale::Th, "OPEN");
    assert_eq!(r.title, TITLE);
    assert_eq!(
        r.body,
        "ยอดนับเปิดกะของคุณ (2026-10-30 10:52) ที่สาขา MAIN-01 ได้รับการรับรองแล้ว"
    );
}

#[test]
fn t1_each_kind_has_its_thai_name() {
    for (kind, name) in [
        ("OPEN", "นับเปิดกะ"),
        ("CLOSE", "นับปิดกะ"),
        ("SAFE_OPEN", "นับเปิดตู้เซฟ"),
        ("SAFE", "นับตู้เซฟ"),
    ] {
        assert_eq!(
            render(Locale::Th, kind).body,
            format!("ยอด{name}ของคุณ (2026-10-30 10:52) ที่สาขา MAIN-01 ได้รับการรับรองแล้ว"),
            "kind {kind}"
        );
    }
}

#[test]
fn t1_every_locale_renders_the_thai_text() {
    let th = render(Locale::Th, "CLOSE");
    for locale in [Locale::En, Locale::Zh] {
        let r = render(locale, "CLOSE");
        assert_eq!(r.title, th.title, "{locale:?} title is Thai");
        assert_eq!(r.body, th.body, "{locale:?} body is Thai");
    }
}
