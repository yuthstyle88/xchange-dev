//! APP-6 (#997) — `AssistantService` and `IntentParser` against a real PostgreSQL:
//! deterministic parsing, text redaction, idempotency, writes no draft/quote/booking,
//! retention-based expiry, sign-in session claim, and owner isolation.
//! Written before the implementation by the test writer (gemini/warut-1, on the BE
//! lane controller claude/ad05bf8d's order); the implementer must not edit, delete
//! or weaken them.
//!
//! Design `docs/design/blueprint/customer-app.md`:
//! - P11: deterministic parser turning text into an intent, named "XCHANGE Assistant",
//!   never presented as AI.
//! - P12: redaction masks phone numbers, OTPs, credentials before storing.
//! - P13: retention data type `ASSISTANT_CONVERSATION`; `expires_at` = `created_at` + period,
//!   or `NULL` if unset.
//! - §4.3: `ask` answers a preview and calls no command; writes no draft, quote, or booking;
//!   the same key in a conversation gives one message; sign-in claims the session's
//!   conversations in one statement; another owner → `outOfScope`.
//! - §9.1 row APP-6: each §9 edge case (except TH/EN/ZH fixture test which awaits prototype zip).

mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use xc_app::application::assistant::assistant_service::AssistantService;
use xc_app::domain::app_error::CustomerAppError;
use xc_app::domain::assistant::conversation::MessageRole;
use xc_app::domain::assistant::intent::{IntentKind, IntentParser};
use xc_app::domain::owner::DraftOwner;
use xc_customer::application::retention_job::RetentionJob;
use xc_customer::domain::retention::DataType;

#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredMessageRow {
    redacted_text: String,
    role: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredConversationRow {
    customer_id: Option<String>,
    session_hash: Option<String>,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
}

#[test]
fn an_unknown_sentence_parses_to_unknown() {
    let en_intent = IntentParser::parse("The weather is really nice today", "en");
    assert_eq!(
        en_intent.kind,
        IntentKind::Unknown,
        "unrelated English sentence must parse to Unknown"
    );
    assert_eq!(en_intent.currency, None);
    assert_eq!(en_intent.customer_side, None);
    assert_eq!(en_intent.quantity, None);
    assert_eq!(en_intent.budget_thb, None);
    assert_eq!(en_intent.branch, None);

    let th_intent = IntentParser::parse("ข้อความทั่วไปที่ไม่เกี่ยวกับธุรกรรมแลกเงิน", "th");
    assert_eq!(
        th_intent.kind,
        IntentKind::Unknown,
        "unrelated Thai sentence must parse to Unknown"
    );
    assert_eq!(th_intent.currency, None);
    assert_eq!(th_intent.customer_side, None);
    assert_eq!(th_intent.quantity, None);
    assert_eq!(th_intent.budget_thb, None);
    assert_eq!(th_intent.branch, None);
}

#[tokio::test]
async fn stored_redacted_text_of_message_holding_phone_and_otp_stores_neither() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let owner = session();

    let text = "My phone is 0812345678 or 081-234-5678, ID is 1100500123456, and OTP is 246810, can I buy 500 USD?";
    let answer = service
        .ask(org(), &owner, "en", text, None, at(0))
        .await
        .expect("ask succeeds");

    let rows: Vec<StoredMessageRow> = sqlx::query_as(
        "SELECT redacted_text, role FROM assistant_messages WHERE conversation_id = $1 ORDER BY created_at ASC",
    )
    .bind(answer.conversation_id)
    .fetch_all(&db.pool)
    .await
    .expect("stored message rows");

    assert!(
        rows.len() >= 2,
        "conversation must store both customer message and assistant reply, got {}",
        rows.len()
    );

    let roles: Vec<&str> = rows.iter().map(|r| r.role.as_str()).collect();
    assert!(
        roles.contains(&MessageRole::Customer.as_str()),
        "customer message must be stored"
    );
    assert!(
        roles.contains(&MessageRole::Assistant.as_str()),
        "assistant message must be stored"
    );

    let sensitive_tokens = ["0812345678", "081-234-5678", "1100500123456", "246810"];
    for row in &rows {
        for token in sensitive_tokens {
            assert!(
                !row.redacted_text.contains(token),
                "stored redacted_text in role {} must not leak token {}, got: {}",
                row.role,
                token,
                row.redacted_text
            );
        }
    }
}

#[tokio::test]
async fn assistant_ask_writes_no_draft_quote_or_booking() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let owner = session();

    let drafts_before = db.count("SELECT count(*) FROM planner_drafts").await;
    let quotes_before = db.count("SELECT count(*) FROM quotes").await;
    let bookings_before = db.count("SELECT count(*) FROM bookings").await;

    let answer = service
        .ask(
            org(),
            &owner,
            "en",
            "I want to exchange 500 USD tomorrow at MAIN",
            None,
            at(0),
        )
        .await
        .expect("ask succeeds");

    let drafts_after = db.count("SELECT count(*) FROM planner_drafts").await;
    let quotes_after = db.count("SELECT count(*) FROM quotes").await;
    let bookings_after = db.count("SELECT count(*) FROM bookings").await;

    assert_eq!(
        drafts_before, drafts_after,
        "assistantAsk must write no planner draft row"
    );
    assert_eq!(
        quotes_before, quotes_after,
        "assistantAsk must write no quote row"
    );
    assert_eq!(
        bookings_before, bookings_after,
        "assistantAsk must write no booking row"
    );

    // Assistant may return a preview to confirm, but does not commit a quote or draft to DB
    assert_eq!(answer.customer_message.quote_id, None);
    assert_eq!(answer.assistant_reply.quote_id, None);
}

#[tokio::test]
async fn the_same_idempotency_key_gives_one_message() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let owner = session();
    let key = "idem-key-app6-replay";

    let first = service
        .ask(
            org(),
            &owner,
            "en",
            "I want to check rates for EUR",
            Some(key),
            at(0),
        )
        .await
        .expect("first ask succeeds");

    let second = service
        .ask(
            org(),
            &owner,
            "en",
            "I want to check rates for EUR",
            Some(key),
            at(5),
        )
        .await
        .expect("second ask with same key replays original message");

    assert_eq!(
        first.conversation_id, second.conversation_id,
        "replayed request returns same conversation"
    );
    assert_eq!(
        first.customer_message.id, second.customer_message.id,
        "replayed request returns same customer message"
    );
    assert_eq!(
        first.assistant_reply.id, second.assistant_reply.id,
        "replayed request returns same assistant reply"
    );

    let count = db
        .count(&format!(
            "SELECT count(*) FROM assistant_messages WHERE conversation_id = '{}' AND idempotency_key = '{key}'",
            first.conversation_id
        ))
        .await;
    assert_eq!(
        count, 1,
        "exactly one message row with the same idempotency key"
    );
}

#[tokio::test]
async fn conversation_expires_at_is_null_when_no_retention_period_is_set() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let owner = session();

    // Ensure no ASSISTANT_CONVERSATION retention policy exists
    db.exec(&format!(
        "DELETE FROM retention_policies WHERE organization_id = '{}' AND data_type = 'ASSISTANT_CONVERSATION'",
        org()
    ))
    .await;

    let answer = service
        .ask(org(), &owner, "en", "Hello assistant", None, at(0))
        .await
        .expect("ask succeeds");

    let conv: StoredConversationRow = sqlx::query_as(
        "SELECT customer_id, session_hash, created_at, expires_at FROM assistant_conversations WHERE id = $1",
    )
    .bind(answer.conversation_id)
    .fetch_one(&db.pool)
    .await
    .expect("conversation row exists");

    assert_eq!(
        conv.expires_at, None,
        "expires_at must be NULL when no period is set"
    );
}

#[tokio::test]
async fn conversation_expires_at_is_set_when_retention_period_is_configured() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let owner = session();

    // Set retention policy for ASSISTANT_CONVERSATION to 30 days
    let retention_job = RetentionJob::pg(db.pool.clone());
    retention_job
        .set_period(ORG, None, DataType::AssistantConversation, Some(30), at(0))
        .await
        .expect("set period");

    let answer = service
        .ask(org(), &owner, "en", "Hello assistant", None, at(0))
        .await
        .expect("ask succeeds");

    let conv: StoredConversationRow = sqlx::query_as(
        "SELECT customer_id, session_hash, created_at, expires_at FROM assistant_conversations WHERE id = $1",
    )
    .bind(answer.conversation_id)
    .fetch_one(&db.pool)
    .await
    .expect("conversation row exists");

    let expires_at = conv.expires_at.expect("expires_at must be set");
    let expected = conv.created_at + Duration::days(30);
    assert_eq!(
        expires_at, expected,
        "expires_at must be created_at + 30 days"
    );
}

#[tokio::test]
async fn claim_session_in_moves_every_session_conversation_and_second_call_moves_none() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let session_owner = session();
    let session_hash = match &session_owner {
        DraftOwner::Session(h) => h.clone(),
        _ => unreachable!(),
    };

    let answer = service
        .ask(
            org(),
            &session_owner,
            "en",
            "I want to exchange currency",
            None,
            at(0),
        )
        .await
        .expect("ask succeeds");

    let customer_id = "CUST-APP6-CLAIM-001";
    db.seed_customer(customer_id).await;

    let mut tx = db.begin().await;
    let moved = service
        .claim_session_in(&mut tx, &session_hash, customer_id, at(10))
        .await
        .expect("claim succeeds");
    assert_eq!(moved, 1, "first claim moves 1 conversation");

    let second_moved = service
        .claim_session_in(&mut tx, &session_hash, customer_id, at(10))
        .await
        .expect("second claim succeeds");
    assert_eq!(second_moved, 0, "second claim moves 0 conversations");
    tx.commit().await.expect("commit");

    // Conversation is now owned by customer_id and session_hash is cleared
    let conv: StoredConversationRow = sqlx::query_as(
        "SELECT customer_id, session_hash, created_at, expires_at FROM assistant_conversations WHERE id = $1",
    )
    .bind(answer.conversation_id)
    .fetch_one(&db.pool)
    .await
    .expect("conversation row");

    assert_eq!(conv.customer_id.as_deref(), Some(customer_id));
    assert_eq!(conv.session_hash, None);
}

#[tokio::test]
async fn a_session_cannot_read_another_session_or_customer_conversation() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let customer_owner = customer("CUST-ALICE-APP6");
    let stranger_session = session();

    db.seed_customer("CUST-ALICE-APP6").await;

    let answer = service
        .ask(
            org(),
            &customer_owner,
            "en",
            "Customer Alice private conversation",
            None,
            at(0),
        )
        .await
        .expect("ask succeeds");

    let res = service
        .get_conversation(&stranger_session, answer.conversation_id)
        .await;

    assert!(
        matches!(res, Err(CustomerAppError::OutOfScope)),
        "a session cannot read a customer's conversation, got: {:?}",
        res
    );

    // Session B cannot read Session A's conversation
    let session_a = session();
    let session_b = session();

    let sess_a_answer = service
        .ask(
            org(),
            &session_a,
            "en",
            "Session A private conversation",
            None,
            at(10),
        )
        .await
        .expect("ask succeeds");

    let res_b = service
        .get_conversation(&session_b, sess_a_answer.conversation_id)
        .await;

    assert!(
        matches!(res_b, Err(CustomerAppError::OutOfScope)),
        "session B cannot read session A's conversation, got: {:?}",
        res_b
    );
}

#[tokio::test]
async fn retention_data_types_round_trip_and_purge() {
    let db = TestDb::start().await;
    let retention_job = RetentionJob::pg(db.pool.clone());

    // 1. DataType round-trips
    assert_eq!(
        DataType::AssistantConversation.as_str(),
        "ASSISTANT_CONVERSATION"
    );
    assert_eq!(
        "ASSISTANT_CONVERSATION".parse::<DataType>().unwrap(),
        DataType::AssistantConversation
    );
    assert_eq!(DataType::PlannerDraft.as_str(), "PLANNER_DRAFT");
    assert_eq!(
        "PLANNER_DRAFT".parse::<DataType>().unwrap(),
        DataType::PlannerDraft
    );

    // 2. Set retention policy: 7 days for ASSISTANT_CONVERSATION
    retention_job
        .set_period(ORG, None, DataType::AssistantConversation, Some(7), at(0))
        .await
        .expect("set period");

    // Seed an expired assistant conversation (expired 10 days ago)
    let conv_id = uuid::Uuid::new_v4();
    let session_h = format!("test_session_hash_{}", unique("ret"));
    let expired_at = at(-10 * 86_400).to_rfc3339();
    let created_at = at(-20 * 86_400).to_rfc3339();
    db.exec(&format!(
        "INSERT INTO assistant_conversations (id, organization_id, session_hash, locale, created_at, expires_at) \
         VALUES ('{conv_id}', '{}', '{session_h}', 'en', '{created_at}', '{expired_at}')",
        org()
    ))
    .await;

    // Purge as of now (at(0))
    let purged = retention_job.purge(at(0)).await.expect("purge runs");
    assert!(
        purged >= 1,
        "purged count should be at least 1, got {}",
        purged
    );

    let count = db
        .count(&format!(
            "SELECT count(*) FROM assistant_conversations WHERE id = '{conv_id}'"
        ))
        .await;
    assert_eq!(count, 0, "expired conversation must be purged");
}
