//! Ticket #212 (follow-up to #186) — pins that an undecodable stored value in any custody family
//! (customer_holds, tx_drafts, cash_exceptions, count sheets, ops_state, requests, signatures)
//! is read as a fault instead of a substitute or default.
//!
//! Re-expected for CA-10 #626 by the ticket author (ruling Q7 A, design `cash-shift-transfer.md`
//! §6): the custody command engine that wrote this history is retired, so the history is written
//! here by SQL, in the shapes `PgCustodyRepository` writes (`custody_repository.rs` `hold_data`,
//! `draft_data`, `exception_data`, `balances_to_json`, `target_to_json`), and
//! `the_seeded_history_reads_as_written` shows it decodes before any value is spoiled. The count
//! sheets lived in `custody_requests`, which M3 drops: their three cases retire with the table.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use custody_service_common::Desk;
use pos_error::{AppError, AppResult};
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

/// A branch with history in every custody table that survives CA-10: the day open with a target
/// (the safe is the ledger SAFE since LG-6.r5 #1044, not custody's), a customer on hold, three
/// drafts, and a cash exception (ten dollars short of the target).
async fn a_branch_with_history(db: &TestDb) -> Desk {
    let d = Desk::new(db).await;
    let at = custody_common::NOON;
    for sql in [
        "INSERT INTO ops_state (branch_id, phase, target) VALUES ($1, 'OPEN', \
         jsonb_build_object('amounts', '{\"THB\": 10000000, \"USD\": 200000}'::jsonb, \
           'by', 'FIN1', 'at', $2::bigint)) \
         ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase, \
           target = EXCLUDED.target",
        "INSERT INTO customer_holds (id, branch_id, position, data) VALUES ('hold-1', $1, 0, \
         jsonb_build_object('customerId', 'CX-00000001', 'reason', 'check', 'by', 'MON1', \
           'at', $2::bigint, 'status', 'OPEN', 'reviewedBy', NULL, 'reviewedAt', NULL, \
           'note', NULL))",
        "INSERT INTO tx_drafts (id, branch_id, position, data) \
         SELECT 'draft-' || n, $1, n - 1, jsonb_build_object('shiftId', 'S1', 'employeeId', 'F1', \
           'side', 'BUY', 'currency', 'USD', 'amount', n * 100, 'at', $2::bigint, \
           'status', 'OPEN', 'documentRef', NULL) \
         FROM generate_series(1, 3) AS n",
        "INSERT INTO cash_exceptions (id, branch_id, position, data) VALUES ('exception-1', $1, 0, \
         jsonb_build_object('requestId', 'count-1', 'status', 'OPEN', \
           'targetDifference', '{\"USD\": -1000}'::jsonb, 'cashVariance', NULL, \
           'safeVariance', NULL, 'note', 'ten dollars short of the target', 'at', $2::bigint, \
           'reviewedBy', NULL, 'reviewedAt', NULL))",
    ] {
        sqlx::query(sql)
            .bind(d.branch.as_str())
            .bind(at)
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    d
}

/// The history as seeded decodes, so a fault below is the spoiled value's and nothing else's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_seeded_history_reads_as_written() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    let state = d
        .custody
        .state(&d.branch)
        .await
        .expect("the seeded history decodes");
    assert_eq!(state.holds.len(), 1, "{state:?}");
    assert_eq!(state.holds[0].customer_id, "CX-00000001");
    assert_eq!(state.drafts.len(), 3, "{state:?}");
    assert_eq!(state.drafts[2].amount.to_string(), "3.00");
    assert_eq!(state.exceptions.len(), 1, "{state:?}");
    let target = state.target.as_ref().expect("the target");
    assert_eq!(
        target.amounts.get(custody_common::code("THB")).to_string(),
        "100000.00"
    );
    db.drop_schema().await;
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

// Ticket #253 — a stored amount is a JSON integer while it fits 64 bits and its decimal text only
// beyond that (`minor_to_json`). A text that would fit 64 bits was never written by the server,
// so reading it is a fault, not the amount it spells. Pins the guard in `minor_from_json`.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_amount_stored_as_text_that_fits_64_bits_is_a_fault() {
    let db = TestDb::start().await;
    let d = a_branch_with_history(&db).await;
    spoil(
        &db,
        "UPDATE tx_drafts SET data = jsonb_set(data, '{amount}', '\"5\"') \
         WHERE branch_id = $1",
        &d,
    )
    .await;
    a_fault(
        d.custody.state(&d.branch).await,
        "a draft amount of \"5\" (text that fits 64 bits)",
    );
    db.drop_schema().await;
}
