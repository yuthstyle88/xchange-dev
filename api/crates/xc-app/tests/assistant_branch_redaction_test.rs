//! APP-6b (#1587) rework — test that branch extraction never leaks PII (such as phone numbers)
//! into `assistant_messages.parsed_intent`.
//!
//! A phone number after "at" / "ที่" / "在" must not become `Intent.branch`.
//! Neither `redacted_text` nor `parsed_intent::text` in the stored row may contain the token.

mod common;

use common::*;
use xc_app::application::assistant::assistant_service::AssistantService;

#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredMessageRow {
    redacted_text: String,
    parsed_intent_text: Option<String>,
}

#[tokio::test]
async fn branch_extraction_does_not_leak_phone_number_into_redacted_text_or_parsed_intent() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let owner = session();

    let test_cases = [
        ("en", "I want 500 USD, call me at 0812345678", "0812345678"),
        (
            "en",
            "I want 500 USD, call me at 081-234-5678",
            "081-234-5678",
        ),
        ("th", "อยากได้ 500 USD โทรหาที่ 0812345678", "0812345678"),
    ];

    for (locale, text, token) in test_cases {
        let answer = service
            .ask(org(), &owner, locale, text, None, at(0))
            .await
            .expect("ask succeeds");

        let row: StoredMessageRow = sqlx::query_as(
            "SELECT redacted_text, parsed_intent::text AS parsed_intent_text \
             FROM assistant_messages \
             WHERE conversation_id = $1 AND role = 'CUSTOMER' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(answer.conversation_id)
        .fetch_one(&db.pool)
        .await
        .expect("stored customer message row exists");

        assert!(
            !row.redacted_text.contains(token),
            "stored redacted_text for {locale} must not contain token {token}, got: {}",
            row.redacted_text
        );

        if let Some(ref parsed_intent_str) = row.parsed_intent_text {
            assert!(
                !parsed_intent_str.contains(token),
                "stored parsed_intent for {locale} must not contain token {token}, got: {}",
                parsed_intent_str
            );
        }
    }

    db.finish().await;
}
