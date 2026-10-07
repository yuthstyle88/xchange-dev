//! Ticket #212 (follow-up to #186) — pins that an undecodable stored value in any custody family
//! (customer_holds, tx_drafts, cash_exceptions, count sheets, ops_state, requests, signatures)
//! is read as a fault instead of a substitute or default.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use custody_service_common::{Desk, rows, totals};
use pos_error::{AppError, AppResult};
use serde_json::json;
use shift_service_common::TestDb;

fn a_fault<T: std::fmt::Debug>(read: AppResult<T>, what: &str) {
    match read {
        Err(AppError::Domain(sentence)) => {
            panic!("{what}: a fault is not a refusal, got Domain({sentence:?})")
        }
        Err(_) => {}
        Ok(value) => panic!("{what}: an undecodable value was read as {value:?}"),
    }
}

/// A branch with history in every table: clocked in, safe counted, drawer opened
/// ten dollars short of target (a cash exception), one transfer completed, one
/// transfer in transit (waiting), a customer on hold, three drafts.
async fn a_branch_with_history(db: &TestDb) -> Desk {
    let mut d = Desk::with_safe(db).await;
    let drawer = [("THB", 100_000), ("USD", 1_990)];
    let open = d
        .propose(
            "F1",
            "OPEN",
            json!({
                "counted": totals(&drawer), "denominations": rows(&drawer),
                "note": "ten dollars short of the target",
            }),
        )
        .await;
    d.ok("BE1", "opsCommit", json!({ "id": open })).await;
    d.ok(
        "BE1",
        "opsStartService",
        json!({ "equipmentReady": true, "ratesVerified": true, "rateUpdated": "2026-10-06T03:00:00.000Z" }),
    )
    .await;

    let done = d
        .propose(
            "F1",
            "SAFE_IN",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": "300.25" }),
        )
        .await;
    d.ok("BE1", "opsDispatch", json!({ "id": done })).await;
    d.ok("FIN1", "opsSign", json!({ "id": done })).await;
    d.ok("BE1", "opsCommit", json!({ "id": done })).await;

    d.outside.add_customer("CX-00000001");
    d.ok(
        "MON1",
        "opsHold",
        json!({ "customerId": "CX-00000001", "reason": "check" }),
    )
    .await;
    for amount in [1, 2, 3] {
        d.ok(
            "F1",
            "opsBeginTransaction",
            json!({ "type": "BUY", "currency": "USD", "amount": amount }),
        )
        .await;
    }

    let waiting = d
        .propose(
            "F1",
            "SAFE_IN",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": 50 }),
        )
        .await;
    d.ok("BE1", "opsDispatch", json!({ "id": waiting })).await;
    d
}

async fn spoil(db: &TestDb, sql: &'static str, d: &Desk) {
    let done = sqlx::query(sql)
        .bind(d.branch.as_str())
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    assert!(done.rows_affected() > 0, "{sql}: no row was spoiled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_customer_hold_status_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE customer_holds SET data = jsonb_set(data, '{status}', '\"MAYBE\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.state(&d.branch).await, "a hold status of MAYBE");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_currency_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE tx_drafts SET data = jsonb_set(data, '{currency}', '\"ZZZ\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.state(&d.branch).await, "a draft currency of ZZZ");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_amount_that_is_not_a_number_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE tx_drafts SET data = jsonb_set(data, '{amount}', '\"lots\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a draft amount of \"lots\"",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_side_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE tx_drafts SET data = jsonb_set(data, '{side}', '\"HOLD\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.state(&d.branch).await, "a draft side of HOLD");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_status_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE tx_drafts SET data = jsonb_set(data, '{status}', '\"MAYBE\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.state(&d.branch).await, "a draft status of MAYBE");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cash_exception_status_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE cash_exceptions SET data = jsonb_set(data, '{status}', '\"MAYBE\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a cash exception status of MAYBE",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_count_sheet_count_that_is_invalid_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_requests SET data = jsonb_set(data, '{counted,denominations,0,count}', '-1') \
         WHERE branch_id = $1 AND data ? 'counted'",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a count-sheet count of -1",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_count_sheet_count_that_overflows_u32_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_requests SET data = jsonb_set(data, '{counted,denominations,0,count}', '4294967296') \
         WHERE branch_id = $1 AND data ? 'counted'",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a count-sheet count of 4294967296",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_count_sheet_denomination_currency_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_requests SET data = jsonb_set(data, '{counted,denominations,0,currency}', '\"ZZZ\"') \
         WHERE branch_id = $1 AND data ? 'counted'",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a count-sheet currency of ZZZ",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ops_state_safe_currency_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE ops_state SET safe = safe || '{\"ZZZ\": 100}' \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(d.custody.state(&d.branch).await, "a safe currency of ZZZ");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ops_state_target_time_that_is_not_a_time_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE ops_state SET target = jsonb_set(target, '{at}', '\"noon\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a target time of \"noon\"",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_stage_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_requests SET data = jsonb_set(data, '{stage}', '\"LOST\"') \
         WHERE branch_id = $1 AND data ? 'stage'",
        &d,
    )
    .await;
    a_fault(d.custody.state(&d.branch).await, "a request stage of LOST");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_reconciliation_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_requests SET data = jsonb_set(data, '{reconciliation}', '\"MAYBE\"') \
         WHERE branch_id = $1 AND data ? 'reconciliation'",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a reconciliation of MAYBE",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_rejected_decision_that_is_invalid_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_requests SET data = jsonb_set(data, '{rejected}', '{\"by\": 1}') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a rejected decision of {\"by\": 1}",
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_custody_signature_role_custody_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE custody_signatures SET role = 'NOBODY' \
         WHERE request_id IN (SELECT id FROM custody_requests WHERE branch_id = $1)",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a signature role of NOBODY",
    );
    db.drop_schema().await;
}
