//! APP-6b (#1587) — `IntentParser` and `AssistantService` planner preview fixture tests.
//!
//! Written before the implementation by the test writer (gemini/warut-1, on the BE
//! lane controller claude/ad05bf8d's order); the implementer must not edit, delete
//! or weaken them.
//!
//! Design:
//! - Blueprint §14, §15 C12: assistant fills the planner, preview before confirm.
//! - Blueprint C12: "regex รับข้อความ TH/EN/ZH → quote/queue/alert".
//! - Design `customer-app.md` P11: deterministic parser turning text into an intent
//!   (`QUOTE`, `QUEUE`, `ALERT`, `BRANCH`, `UNKNOWN`) and a preview (a planner draft
//!   estimate the customer confirms, or None).
//! - Ruling B (claude/ad05bf8d, #1587): fixture in TH/EN/ZH with at least 4 QUOTE sentences
//!   (amount + currency, with and without branch/day), 2 QUEUE, 2 ALERT, and 3 UNKNOWN.
//! - Ruling A (claude/ad05bf8d): for every QUOTE row, `preview == PlannerPreview::estimate(plan from the parsed intent)`,
//!   and `None` for UNKNOWN.

mod common;

use common::*;
use serde::Deserialize;
use xc_app::application::assistant::assistant_service::AssistantService;
use xc_app::application::planner::planner_preview::PlannerPreview;
use xc_app::domain::assistant::intent::{Intent, IntentKind, IntentParser};
use xc_app::domain::planner::planner_draft::{DraftAmount, DraftPlan};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

#[derive(Debug, Clone, Deserialize)]
struct IntentFixtureRow {
    th: String,
    en: String,
    zh: String,
    intent: FixtureIntent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureIntent {
    kind: String,
    currency: Option<String>,
    customer_side: Option<String>,
    quantity: Option<String>,
    budget_thb: Option<String>,
    branch: Option<String>,
}

impl FixtureIntent {
    fn to_intent(&self) -> Intent {
        let kind = match self.kind.as_str() {
            "QUOTE" => IntentKind::Quote,
            "QUEUE" => IntentKind::Queue,
            "ALERT" => IntentKind::Alert,
            "BRANCH" => IntentKind::Branch,
            _ => IntentKind::Unknown,
        };
        let currency = self
            .currency
            .as_deref()
            .and_then(|s| CurrencyCode::parse(s).ok());
        let customer_side = self.customer_side.as_deref().and_then(CustomerSide::parse);
        let quantity = self
            .quantity
            .as_deref()
            .and_then(|s| s.parse::<rust_decimal::Decimal>().ok())
            .map(ForeignAmount::from_decimal);
        let budget_thb = self
            .budget_thb
            .as_deref()
            .and_then(|s| Money::parse(s).ok());
        let branch = self.branch.as_deref().map(BranchId::new);

        Intent {
            kind,
            currency,
            customer_side,
            quantity,
            budget_thb,
            branch,
        }
    }
}

fn load_fixtures() -> Vec<IntentFixtureRow> {
    let raw = include_str!("fixtures/assistant_intents.json");
    serde_json::from_str(raw).expect("valid assistant_intents.json fixture")
}

#[test]
fn each_fixture_sentence_in_th_en_and_zh_parses_to_the_same_intent() {
    let rows = load_fixtures();
    assert!(
        rows.len() >= 11,
        "expected at least 11 fixture rows, got {}",
        rows.len()
    );

    let quote_count = rows.iter().filter(|r| r.intent.kind == "QUOTE").count();
    let queue_count = rows.iter().filter(|r| r.intent.kind == "QUEUE").count();
    let alert_count = rows.iter().filter(|r| r.intent.kind == "ALERT").count();
    let unknown_count = rows.iter().filter(|r| r.intent.kind == "UNKNOWN").count();

    assert!(
        quote_count >= 4,
        "expected at least 4 QUOTE rows, got {quote_count}"
    );
    assert!(
        queue_count >= 2,
        "expected at least 2 QUEUE rows, got {queue_count}"
    );
    assert!(
        alert_count >= 2,
        "expected at least 2 ALERT rows, got {alert_count}"
    );
    assert!(
        unknown_count >= 3,
        "expected at least 3 UNKNOWN rows, got {unknown_count}"
    );

    for row in &rows {
        let expected = row.intent.to_intent();

        let th_parsed = IntentParser::parse(&row.th, "th");
        assert_eq!(
            th_parsed, expected,
            "TH sentence '{}' failed to parse to expected intent",
            row.th
        );

        let en_parsed = IntentParser::parse(&row.en, "en");
        assert_eq!(
            en_parsed, expected,
            "EN sentence '{}' failed to parse to expected intent",
            row.en
        );

        let zh_parsed = IntentParser::parse(&row.zh, "zh");
        assert_eq!(
            zh_parsed, expected,
            "ZH sentence '{}' failed to parse to expected intent",
            row.zh
        );
    }
}

#[tokio::test]
async fn a_quote_sentence_answers_preview_matching_planner_preview_estimate() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD, JPY]).await;

    let service = AssistantService::new(db.pool.clone());
    let preview = PlannerPreview::new(db.pool.clone());
    let rows = load_fixtures();
    let quote_rows: Vec<_> = rows
        .into_iter()
        .filter(|r| r.intent.kind == "QUOTE")
        .collect();
    assert!(!quote_rows.is_empty(), "expected QUOTE rows in fixture");

    for row in &quote_rows {
        let intent = row.intent.to_intent();
        let plan = DraftPlan {
            branch_id: intent.branch.clone(),
            currency: intent.currency.expect("QUOTE intent must have currency"),
            customer_side: intent
                .customer_side
                .expect("QUOTE intent must have customer side"),
            amount: DraftAmount::Quantity(
                intent.quantity.expect("QUOTE intent must have quantity"),
            ),
        };

        let now = at(0);
        let mut tx = db.pool.begin().await.expect("begin tx");
        let expected_preview = preview.estimate(&mut tx, &plan, now).await.ok();
        tx.rollback().await.expect("rollback");

        if intent.branch.is_some() {
            assert!(
                expected_preview.is_some(),
                "expected preview must be Some for quote plan with branch {:?}",
                plan
            );
        } else {
            assert!(
                expected_preview.is_none(),
                "expected preview must be None for quote plan without branch"
            );
        }

        for (locale, text) in [("en", &row.en), ("th", &row.th), ("zh", &row.zh)] {
            let answer = service
                .ask(org(), &session(), locale, text, None, now)
                .await
                .expect("ask succeeds");

            assert_eq!(
                answer.preview, expected_preview,
                "preview mismatch for {locale} sentence '{text}'"
            );
        }
    }

    db.finish().await;
}

#[tokio::test]
async fn an_unknown_sentence_answers_unknown_intent_and_preview_is_none() {
    let db = TestDb::start().await;
    let service = AssistantService::new(db.pool.clone());
    let rows = load_fixtures();
    let unknown_rows: Vec<_> = rows
        .into_iter()
        .filter(|r| r.intent.kind == "UNKNOWN")
        .collect();
    assert!(!unknown_rows.is_empty(), "expected UNKNOWN rows in fixture");

    for row in &unknown_rows {
        for (locale, text) in [("en", &row.en), ("th", &row.th), ("zh", &row.zh)] {
            let answer = service
                .ask(org(), &session(), locale, text, None, at(0))
                .await
                .expect("ask succeeds");

            assert_eq!(
                answer.customer_message.parsed_intent,
                Some(Intent::unknown()),
                "parsed intent must be unknown for {locale} sentence '{text}'"
            );
            assert_eq!(
                answer.preview, None,
                "preview must be None for {locale} unknown sentence '{text}'"
            );
        }
    }

    db.finish().await;
}
