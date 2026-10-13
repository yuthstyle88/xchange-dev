//! APP-10 (#1001, design `docs/design/blueprint/customer-app.md` P8–P10, §4.1, §5, §9.1 row
//! APP-10): the `queue_called` template in `src/templates/queue_called.rs`. Written before the
//! implementation by the ticket author (claude/89bd61d9). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! What this file fixes (`[required]` once pushed):
//! - the key is `queue_called`, the template is `TemplateKind::Transactional` (§4.1: a queue call
//!   needs no marketing consent), and it is found in `TemplateRegistry::generated()`;
//! - its params are `displayNumber` and `branchName` (the shape APP-3's harness already used for
//!   `displayNumber`); each locale's text shows both values, and TH, EN and ZH differ (owner C2).
//!
//! The database test needs `TEST_DATABASE_URL` naming an existing database ending in `_test`.

mod common;

use std::sync::Arc;

use common::*;

const KEY: &str = "queue_called";

fn queue_params() -> std::collections::BTreeMap<String, String> {
    params(&[("displayNumber", "W007"), ("branchName", "XC-Siam")])
}

#[test]
fn the_generated_registry_has_queue_called_as_a_transactional_template() {
    let registry = TemplateRegistry::generated();
    let template = registry
        .get(KEY)
        .expect("src/templates/queue_called.rs is generated into the registry");
    assert_eq!(template.key(), KEY);
    assert_eq!(template.kind(), TemplateKind::Transactional);
    assert!(
        template.version() >= 1,
        "a stored template version is 1 or more"
    );
}

#[test]
fn every_locale_shows_the_display_number_and_the_branch_name() {
    let registry = TemplateRegistry::generated();
    let template = registry.get(KEY).expect("queue_called is registered");
    for locale in [Locale::Th, Locale::En, Locale::Zh] {
        let r = template.render(locale, &queue_params());
        assert!(
            !r.title.trim().is_empty(),
            "{locale:?}: the title is not empty"
        );
        let text = format!("{} {}", r.title, r.body);
        assert!(
            text.contains("W007"),
            "{locale:?}: the display number is shown, got {text:?}"
        );
        assert!(
            text.contains("XC-Siam"),
            "{locale:?}: the branch name is shown, got {text:?}"
        );
    }
}

#[test]
fn thai_english_and_chinese_are_three_different_texts() {
    let registry = TemplateRegistry::generated();
    let template = registry.get(KEY).expect("queue_called is registered");
    let th = template.render(Locale::Th, &queue_params());
    let en = template.render(Locale::En, &queue_params());
    let zh = template.render(Locale::Zh, &queue_params());
    assert_ne!(th, en, "TH and EN are translated, not one text");
    assert_ne!(th, zh, "TH and ZH are translated, not one text");
    assert_ne!(en, zh, "EN and ZH are translated, not one text");
}

#[test]
fn the_text_follows_the_params_rather_than_a_fixed_number() {
    let registry = TemplateRegistry::generated();
    let template = registry.get(KEY).expect("queue_called is registered");
    let other = params(&[("displayNumber", "B012"), ("branchName", "XC-Chidlom")]);
    for locale in [Locale::Th, Locale::En, Locale::Zh] {
        let r = template.render(locale, &other);
        let text = format!("{} {}", r.title, r.body);
        assert!(
            text.contains("B012") && text.contains("XC-Chidlom"),
            "{locale:?}: rendered from the params, got {text:?}"
        );
        assert!(
            !text.contains("W007") && !text.contains("XC-Siam"),
            "{locale:?}: no value of another call leaks in, got {text:?}"
        );
    }
}

#[tokio::test]
async fn notify_in_with_the_generated_registry_writes_a_sent_in_app_delivery_without_marketing_consent()
 {
    let db = TestDb::start().await;
    let service =
        NotificationService::new(db.pool.clone(), Arc::new(TemplateRegistry::generated()));
    // A customer with no consents row at all: a transactional template is still delivered (§4.1).
    let (user, _) = db.seed_customer_user("zh").await;
    let req = NotificationRequest::new(
        user.clone(),
        "queue.called:T-1:1",
        KEY,
        None,
        queue_params(),
    )
    .expect("the queue params hold no personal data");

    let id = db.notify(&service, &req).await;

    let row = db.notification(&id).await;
    assert_eq!(row.template_key, KEY);
    assert_eq!(row.recipient_user_id, user);
    assert_eq!(row.locale, "zh");
    let deliveries = db.deliveries(&id).await;
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].channel, "IN_APP");
    assert_eq!(deliveries[0].status, "SENT");
    assert_eq!(deliveries[0].skipped_reason, None);
    db.finish().await;
}
