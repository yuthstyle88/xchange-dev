//! Tests for SM-1 (#1490, `rates.md` §12.8, §12.10): rate_sharp_move notify template.
//! Written before the implementation by the test-writer (gemini/pool-1).
//!
//! §9 Edge cases:
//! - Row 6: The template renders TH and EN titles and bodies, both non-empty.
//! - Row 7: NotificationRequest::new accepts the template's params for a realistic branch code (e.g. MAIN, BKK01), proving the params pass redact_text unchanged.

use std::collections::BTreeMap;
use xc_kernel::notifications::NotificationRequest;
use xc_notify::application::template_registry::TemplateRegistry;
use xc_notify::domain::template::{Locale, TemplateKind};

#[test]
fn case_6_the_template_renders_th_and_en_titles_and_bodies_both_non_empty() {
    let registry = TemplateRegistry::generated();
    let template = registry
        .get("rate_sharp_move")
        .expect("rate_sharp_move must be registered in TemplateRegistry::generated()");

    assert_eq!(template.key(), "rate_sharp_move");
    assert_eq!(template.kind(), TemplateKind::Transactional);
    assert_eq!(template.version(), 1);

    let mut params = BTreeMap::new();
    params.insert("branch".to_string(), "MAIN".to_string());
    params.insert("currency".to_string(), "USD".to_string());
    params.insert("side".to_string(), "BUY".to_string());

    // TH rendering
    let rendered_th = template.render(Locale::Th, &params);
    assert!(
        !rendered_th.title.trim().is_empty(),
        "TH title must not be empty"
    );
    assert!(
        !rendered_th.body.trim().is_empty(),
        "TH body must not be empty"
    );
    assert!(
        rendered_th.body.contains("MAIN"),
        "TH body should mention branch: {}",
        rendered_th.body
    );
    assert!(
        rendered_th.body.contains("USD"),
        "TH body should mention currency: {}",
        rendered_th.body
    );

    // EN rendering
    let rendered_en = template.render(Locale::En, &params);
    assert!(
        !rendered_en.title.trim().is_empty(),
        "EN title must not be empty"
    );
    assert!(
        !rendered_en.body.trim().is_empty(),
        "EN body must not be empty"
    );
    assert!(
        rendered_en.body.contains("MAIN"),
        "EN body should mention branch: {}",
        rendered_en.body
    );
    assert!(
        rendered_en.body.contains("USD"),
        "EN body should mention currency: {}",
        rendered_en.body
    );
}

#[test]
fn case_7_notification_request_new_accepts_template_params_for_realistic_branch_code() {
    for branch_code in ["MAIN", "BKK01"] {
        let mut params = BTreeMap::new();
        params.insert("branch".to_string(), branch_code.to_string());
        params.insert("currency".to_string(), "USD".to_string());
        params.insert("side".to_string(), "BUY".to_string());

        let req = NotificationRequest::new(
            "user-1",
            format!("rate-sharp-move:pub-1:USD:BUY:{branch_code}"),
            "rate_sharp_move",
            None,
            params.clone(),
        );

        assert!(
            req.is_ok(),
            "NotificationRequest::new must accept params without numbers: {req:?}"
        );

        for (k, v) in &params {
            assert_eq!(
                xc_kernel::pii::redact_text(v),
                *v,
                "param '{k}' with value '{v}' must pass redact_text unchanged"
            );
        }
    }
}
