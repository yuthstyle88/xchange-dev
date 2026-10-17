//! #2881 §2 — the Loyalty service's own error text never reaches an XC client.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-4). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Each case makes the in-process fake writer (`loyalty_fake_writer`) fail with a distinctive
//! upstream message, runs the v3 command directly, and checks the `V3Error` it returns: the `code`
//! and `status` stay what they are today, and the `message` (what the reply body carries) never
//! contains the upstream text. The upstream text may be logged; that is not asserted here.

mod loyalty_fake_writer;
mod quote_common;

use actix_web::http::StatusCode;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_loyalty::application::rule_publisher::LoyaltyRuleError;
use xchange::bootstrap::approval_subjects::ApprovalSubjectDeps;
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::presentation::http::v3::approval_decide::{
    ApprovalDecideCommand, ApprovalDecideInput,
};
use xchange::presentation::http::v3::command::CommandAction;
use xchange::presentation::http::v3::commands::make_approval_service_with;
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::http::v3::loyalty_rule_draft::{
    LoyaltyRuleDraftCommand, LoyaltyRuleDraftInput,
};
use xchange::presentation::http::v3::loyalty_rule_submit::{
    LoyaltyRuleSubmitCommand, LoyaltyRuleSubmitInput,
};

use loyalty_fake_writer::FakeWriter;
use quote_common::{ORG_1, TestDb};

/// What the Loyalty service might put in `error.message`, or what reqwest puts in an error string.
const UPSTREAM: &str = "UPSTREAM-2881 row 7f3a9c at http://loyalty.internal:8443/v1";
const MAIN: &str = "MAIN";

fn executive(user: &str) -> Actor {
    Actor::new(
        user,
        ORG_1,
        vec![ScopedRole::new(
            Role::Executive,
            None,
            chrono::DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

fn key() -> IdempotencyKey {
    IdempotencyKey::new(Uuid::new_v4())
}

#[track_caller]
fn assert_clean(err: &V3Error, code: &str, status: StatusCode) {
    assert_eq!(err.code, code, "the code stays as today: {err:?}");
    assert_eq!(err.status, status, "the status stays as today: {err:?}");
    assert!(
        !err.message.contains("UPSTREAM-2881") && !err.message.contains("loyalty.internal"),
        "the reply message carries Loyalty's own text: {:?}",
        err.message
    );
    assert!(
        !err.message.trim().is_empty(),
        "the reply still says something"
    );
}

async fn draft_error(writer: &FakeWriter) -> V3Error {
    let db = TestDb::empty().await;
    let mut tx = db.pool.begin().await.expect("begin");
    let err = LoyaltyRuleDraftCommand::new(Some(writer.as_writer()))
        .run(
            &mut tx,
            &BranchId::new(MAIN),
            &executive("user-EXECUTIVE"),
            LoyaltyRuleDraftInput {
                kind: "POINT".to_owned(),
                fields: json!({ "pointsRatio": "1", "minimumBasisAmount": "100" }),
            },
            key(),
        )
        .await
        .expect_err("the fake refuses the draft");
    drop(tx);
    db.finish().await;
    err
}

async fn submit_error(db: &TestDb, writer: &FakeWriter) -> V3Error {
    let deps = ApprovalSubjectDeps {
        pool: db.pool.clone(),
        loyalty: Some(writer.as_writer()),
    };
    let mut tx = db.pool.begin().await.expect("begin");
    let err =
        LoyaltyRuleSubmitCommand::new(Some(writer.as_writer()), make_approval_service_with(&deps))
            .run(
                &mut tx,
                &BranchId::new(MAIN),
                &executive("user-EXECUTIVE"),
                LoyaltyRuleSubmitInput {
                    version_id: writer.id.to_string(),
                },
                key(),
            )
            .await
            .expect_err("the fake refuses the submit");
    drop(tx);
    err
}

#[tokio::test]
async fn a_draft_while_loyalty_is_down_does_not_show_loyaltys_text() {
    let writer = FakeWriter::new();
    writer.fail_create(LoyaltyRuleError::Unavailable(UPSTREAM.to_owned()));
    let err = draft_error(&writer).await;
    assert_clean(&err, "loyaltyUnavailable", StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_draft_loyalty_refuses_does_not_show_loyaltys_text() {
    let writer = FakeWriter::new();
    writer.fail_create(LoyaltyRuleError::Invalid(UPSTREAM.to_owned()));
    let err = draft_error(&writer).await;
    assert_clean(&err, "validationFailed", StatusCode::BAD_REQUEST);
    assert_eq!(err.field, Some("fields"), "the field stays as today");
}

#[tokio::test]
async fn a_submit_while_loyalty_is_down_does_not_show_loyaltys_text() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    writer.fail_get(LoyaltyRuleError::Unavailable(UPSTREAM.to_owned()));
    let err = submit_error(&db, &writer).await;
    assert_clean(&err, "loyaltyUnavailable", StatusCode::SERVICE_UNAVAILABLE);
    db.finish().await;
}

#[tokio::test]
async fn a_submit_loyalty_refuses_to_move_does_not_show_loyaltys_text() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    writer.fail_submit(LoyaltyRuleError::InvalidTransition(UPSTREAM.to_owned()));
    let err = submit_error(&db, &writer).await;
    assert_clean(&err, "staleVersion", StatusCode::CONFLICT);
    db.finish().await;
}

#[tokio::test]
async fn a_submit_loyalty_finds_invalid_does_not_show_loyaltys_text() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    writer.fail_submit(LoyaltyRuleError::Invalid(UPSTREAM.to_owned()));
    let err = submit_error(&db, &writer).await;
    assert_clean(&err, "validationFailed", StatusCode::BAD_REQUEST);
    db.finish().await;
}

#[tokio::test]
async fn a_decide_while_loyalty_is_down_does_not_show_loyaltys_text() {
    let db = TestDb::start().await;
    let writer = FakeWriter::new();
    let deps = ApprovalSubjectDeps {
        pool: db.pool.clone(),
        loyalty: Some(writer.as_writer()),
    };
    let service = make_approval_service_with(&deps);

    // A submitted version with its open request.
    let mut tx = db.pool.begin().await.expect("begin");
    let submitted = LoyaltyRuleSubmitCommand::new(Some(writer.as_writer()), service.clone())
        .run(
            &mut tx,
            &BranchId::new(MAIN),
            &executive("user-EXECUTIVE"),
            LoyaltyRuleSubmitInput {
                version_id: writer.id.to_string(),
            },
            key(),
        )
        .await
        .expect("the submit works while Loyalty answers");
    tx.commit().await.expect("commit the submit");

    // Loyalty goes down before another executive decides.
    writer.fail_get(LoyaltyRuleError::Unavailable(UPSTREAM.to_owned()));
    let mut tx = db.pool.begin().await.expect("begin");
    let err = ApprovalDecideCommand::new(service)
        .run(
            &mut tx,
            &BranchId::new(MAIN),
            &executive("user-EXECUTIVE-2"),
            ApprovalDecideInput {
                id: submitted.request_id.clone(),
                outcome: "APPROVED".to_owned(),
                reason: "ok".to_owned(),
            },
            key(),
        )
        .await
        .expect_err("the decide cannot reach Loyalty");
    drop(tx);
    assert_clean(&err, "loyaltyUnavailable", StatusCode::SERVICE_UNAVAILABLE);
    db.finish().await;
}
