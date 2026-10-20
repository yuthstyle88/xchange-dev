//! S4 of the FE survey section B (#3407; design `docs/design/blueprint/section-b-gaps.md` §S4, PR 3387;
//! `customer-app.md` P11: the assistant answers an intent and a preview).
//!
//! 🔴 Written by the ticket writer claude/studio-claude-4 before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `POST /api/app/assistant/messages` answers, besides its three fields, `intent`
//! (`{kind, currency?, customerSide?, quantity?, budgetThb?, branch?}`) and `preview`
//! (`PlannerPreviewAnswer` or `null`).
//!
//! Harness: `quote_common`. Sentences: `crates/xc-app/tests/fixtures/assistant_intents.json`.
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod common;
mod quote_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::{Line, TestDb, unique};
use serde_json::{Value, json};
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xchange::bootstrap::{self, AppState};

const MAIN: &str = "MAIN";
/// The shop sells USD at 35.50, so a customer buying 500 USD pays 17 750.00 THB.
const USD: Line = Line::manual("USD", "1", "35.10", "35.50");

struct Reply {
    status: StatusCode,
    text: String,
    body: Value,
}

/// One anonymous `assistantAsk` on a fresh app session.
async fn ask(db: &TestDb, text: &str) -> Reply {
    common::install_test_pii_keys();
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let sid = AppSessionId::generate().as_str().to_owned();
    let req = atest::TestRequest::post()
        .uri("/api/app/assistant/messages")
        .insert_header((header::HOST, "xc.test"))
        .insert_header((header::ORIGIN, "http://xc.test"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::COOKIE, format!("{APP_SESSION_COOKIE}={sid}")))
        .insert_header(("idempotency-key", unique("s4-ask")))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .set_payload(json!({ "branch": MAIN, "text": text, "locale": "th" }).to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Reply { status, text, body }
}

fn number(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("a decimal string: {v}"))
        .parse()
        .unwrap_or_else(|e| panic!("{v}: {e}"))
}

/// B1: a QUOTE with a published line answers its intent and the planner's estimate.
#[tokio::test]
async fn s4_a_quote_answers_its_intent_and_a_preview() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;

    let r = ask(&db, "อยากได้ 500 USD พรุ่งนี้ที่ MAIN").await;

    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["intent"]["kind"], "QUOTE", "{}", r.text);
    assert_eq!(r.body["intent"]["currency"], "USD", "{}", r.text);
    assert_eq!(
        r.body["intent"]["customerSide"], "CUSTOMER_BUY",
        "{}",
        r.text
    );
    assert_eq!(number(&r.body["intent"]["quantity"]), 500.0, "{}", r.text);
    assert_eq!(number(&r.body["preview"]["rate"]), 35.50, "{}", r.text);
    assert_eq!(number(&r.body["preview"]["thb"]), 17_750.0, "{}", r.text);
    assert_eq!(r.body["preview"]["stale"], false, "{}", r.text);
    db.finish().await;
}

/// B2: a QUOTE for a currency with no published line has its intent and no preview.
#[tokio::test]
async fn s4_a_quote_with_no_published_line_answers_a_null_preview() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;

    let r = ask(&db, "ต้องการขาย 20000 JPY พรุ่งนี้ที่ MAIN").await;

    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["intent"]["kind"], "QUOTE", "{}", r.text);
    assert_eq!(r.body["intent"]["currency"], "JPY", "{}", r.text);
    assert!(
        r.body.get("preview").is_some() && r.body["preview"].is_null(),
        "preview is present and null: {}",
        r.text
    );
    db.finish().await;
}

/// B3 + B4: a queue question and an unknown sentence answer their kind and no preview.
#[tokio::test]
async fn s4_queue_and_unknown_sentences_answer_their_kind_and_no_preview() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;

    for (text, kind) in [("ตรวจสอบสถานะคิว", "QUEUE"), ("วันนี้อากาศดีมากเลย", "UNKNOWN")]
    {
        let r = ask(&db, text).await;
        assert_eq!(r.status, StatusCode::OK, "{text}: {}", r.text);
        assert_eq!(r.body["intent"]["kind"], kind, "{text}: {}", r.text);
        assert!(
            r.body.get("preview").is_some() && r.body["preview"].is_null(),
            "{text}: preview is present and null: {}",
            r.text
        );
    }
    db.finish().await;
}

/// B5: nothing new is stored; the conversation still holds only redacted text.
#[tokio::test]
async fn s4_answering_the_intent_stores_nothing_new() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD]).await;
    let drafts_before = db.count("SELECT count(*) FROM planner_drafts").await;
    let quotes_before = db.count("SELECT count(*) FROM quotes").await;

    let r = ask(&db, "อยากได้ 500 USD พรุ่งนี้ที่ MAIN").await;

    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        db.count("SELECT count(*) FROM planner_drafts").await,
        drafts_before
    );
    assert_eq!(db.count("SELECT count(*) FROM quotes").await, quotes_before);
    db.finish().await;
}
