//! #5397 (C5): the `count_rejected` notification template, sent to the person whose cash count was rejected.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-16). Do not edit, delete or
//! weaken these tests; add your own in another file. No test here needs a database.
//!
//! The template is one new file, `src/templates/count_rejected.rs` (`KEY = "count_rejected"`, transactional).
//! Its params (set by `xc-cash`'s `reject_in`, pinned in `xc-cash/tests/count_reject_notice_test.rs`):
//! `kind` (`OPEN`, `CLOSE`, `SAFE_OPEN`, `SAFE`), `branch`, `counted_on` (`YYYY-MM-DD`), `counted_time`
//! (`HH:MM`), `reason`.
//!
//! Server texts are Thai only in feature work (`docs/design/walkin-queue.md:15`), so every locale renders the same
//! Thai title and body. The kind is named as the staff screen names its button (`นับเปิดกะ`, `นับปิดกะ`).

mod common;

use common::*;

const TITLE: &str = "ยอดนับเงินถูกปฏิเสธ";

fn rejected(kind: &str) -> std::collections::BTreeMap<String, String> {
    params(&[
        ("kind", kind),
        ("branch", "MAIN-01"),
        ("counted_on", "2026-10-28"),
        ("counted_time", "12:30"),
        ("reason", "ยอด USD ไม่ตรง นับใหม่"),
    ])
}

fn render(locale: Locale, kind: &str) -> Rendered {
    let registry = TemplateRegistry::generated();
    let template = registry
        .get("count_rejected")
        .expect("the count_rejected template");
    template.render(locale, &rejected(kind))
}

#[test]
fn the_template_is_registered_and_transactional() {
    let registry = TemplateRegistry::generated();
    let template = registry
        .get("count_rejected")
        .expect("the count_rejected template");
    assert_eq!(template.key(), "count_rejected");
    assert_eq!(template.kind(), TemplateKind::Transactional);
}

#[test]
fn n2_the_thai_text_names_the_kind_time_branch_and_reason() {
    let r = render(Locale::Th, "OPEN");
    assert_eq!(r.title, TITLE);
    assert_eq!(
        r.body,
        "ยอดนับเปิดกะของคุณ (2026-10-28 12:30) ที่สาขา MAIN-01 ถูกปฏิเสธ: ยอด USD ไม่ตรง นับใหม่ — กรุณานับใหม่"
    );
}

#[test]
fn n2_each_kind_has_its_thai_name() {
    for (kind, name) in [
        ("OPEN", "นับเปิดกะ"),
        ("CLOSE", "นับปิดกะ"),
        ("SAFE_OPEN", "นับเปิดตู้เซฟ"),
        ("SAFE", "นับตู้เซฟ"),
    ] {
        assert_eq!(
            render(Locale::Th, kind).body,
            format!(
                "ยอด{name}ของคุณ (2026-10-28 12:30) ที่สาขา MAIN-01 ถูกปฏิเสธ: ยอด USD ไม่ตรง นับใหม่ — กรุณานับใหม่"
            ),
            "kind {kind}"
        );
    }
}

#[test]
fn n2_every_locale_renders_the_thai_text() {
    let th = render(Locale::Th, "CLOSE");
    for locale in [Locale::En, Locale::Zh] {
        let r = render(locale, "CLOSE");
        assert_eq!(r.title, th.title, "{locale:?} title is Thai");
        assert_eq!(r.body, th.body, "{locale:?} body is Thai");
    }
}
