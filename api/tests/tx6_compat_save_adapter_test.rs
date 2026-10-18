//! Written before the implementation (TX-6 #674, design `docs/design/blueprint/quote-transaction.md` §4.6,
//! §9.1 TX-6). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The compat `saveTransaction` is an adapter over quote → create → confirm → complete in **one**
//! `BranchTx`, keyed by today's `submissionId`, all-or-nothing (D-1). Only the rows that hold
//! whatever §4.6 later decides for the DEMO, kiosk, booking and direct-workflow paths are here
//! (lane controller's ruling (b) on #674): every save is LIVE, walk-in, through
//! `POST /api/rpc`, so no constructor and no `run_in` signature is pinned.
//!
//! The day of `rpc/engine_actions.json` is replayed through the routes up to #119 (no exchange is
//! saved by then); then the day's FRONT, FR1 (a TELLER, OR-4b #767 C2), sends the saves under test.
//!
//! Not here, on purpose:
//! - **"today's provided save tests pass byte for byte"**: those are the provided tests themselves
//!   (`api/tests/*save*`, `rpc_test.rs`, `fixtures/save/v2_save_cases.json`);
//! - **an expired quote**: a compat save quotes and confirms at one instant, and
//!   `exchange_settings.quote_ttl_seconds` is `CHECK (BETWEEN 30 AND 3600)`
//!   (`migrations/20261014100000_quotes.sql:46`), so a compat save cannot meet `quoteExpired`;
//! - **"only TX-2's store writes `transactions`"** (#674 criterion 3): waits for the design fix of
//!   §4.6 (gap 4 on #674).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod shift_holder_common;
mod snapshot_common;

use compat_common::Fixture;
use rpc_common::{Answer, Call, Desk, RPC, Replay};
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const FILE: &str = "rpc/engine_actions.json";
/// The day's FRONT: B1 makes FRONT a TELLER, whose `ExchangeTransact` the save is (OR-4b C2).
const TELLER: &str = "FR1";

/// `engine_actions.json`: #114 a LIVE SELL of 750 USD the shift has not got, #119 the last step
/// before the day's first save, #120 a LIVE BUY of 100 USD by cash at 32.2 (`XC-20261006-000001`).
const NOT_ENOUGH_USD_AT: usize = 114;
const LAST_BEFORE_THE_SAVE: usize = 119;
const LIVE_BUY_CASH: usize = 120;
const FIRST_OF_THE_DAY: &str = "XC-20261006-000001";

const NOT_ENOUGH_USD: &str = "ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน";
const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";
const UNAVAILABLE: &str = "Payment method unavailable. Please contact staff.";
/// The passport number of `CX-00000001` as #120 sends it.
const PASSPORT: &str = "P1000001";

/// The day replayed through the routes up to and including #119.
async fn day_up_to_the_save(fixture: &Fixture) -> Replay<'_> {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
    for n in 1..=LAST_BEFORE_THE_SAVE {
        replay.step(fixture.step(n)).await;
    }
    replay
}

/// The payload step `n` sent, with the ids v3 made in place of v2's, and the clock at its time.
fn payload_of(replay: &Replay<'_>, n: usize) -> Value {
    let step = replay.fixture.step(n);
    replay.desk.clock.set(time(&step.at));
    replay
        .ids
        .to_actual(step.payload.as_ref().expect("a recorded payload"))
}

/// `saveTransaction` on `/api/rpc` as the teller, with the teller's own copy of the files.
async fn save(replay: &Replay<'_>, payload: &Value) -> Answer {
    let payload = replay.uploads_as(TELLER, payload).await;
    replay
        .app
        .send(Call::action(ORIGIN, RPC, "saveTransaction", payload).cookie(&replay.cookies[TELLER]))
        .await
}

/// `(quotes, transactions, approval_requests)` of `MAIN`.
async fn rows(replay: &Replay<'_>) -> (i64, i64, i64) {
    let pool = &replay.desk.db.pool;
    let count = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    };
    (
        count("SELECT count(*) FROM quotes WHERE branch_id = 'MAIN'").await,
        count("SELECT count(*) FROM transactions WHERE branch_id = 'MAIN'").await,
        count("SELECT count(*) FROM approval_requests WHERE branch_id = 'MAIN'").await,
    )
}

const NONE: (i64, i64, i64) = (0, 0, 0);

/// The error of a refused answer, which must be a 400.
fn refused(answer: &Answer, what: &str) -> String {
    assert_eq!(answer.status, 400, "{what}: {:.300}", answer.text);
    answer.error()
}

// ── success ──────────────────────────────────────────────────────────────────

/// A compat save is quoted, created, confirmed and completed: its exchange is COMPLETED and bound
/// to a quote of its own, which is ACCEPTED by that exchange (§4.6, §4.4 confirm step 4). The
/// answer is the one v2 gave.
#[actix_web::test]
async fn a_compat_save_is_completed_through_a_quote_it_accepted() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    shift_holder_common::open_day(&replay.desk.db.pool, "MAIN").await;
    assert_eq!(
        rows(&replay).await,
        NONE,
        "no exchange before the day's first save"
    );

    let answer = save(&replay, &payload_of(&replay, LIVE_BUY_CASH)).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
    assert_eq!(answer.json()["documentRef"], json!(FIRST_OF_THE_DAY));

    let (status, quote): (String, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT status, quote_id FROM transactions WHERE branch_id = 'MAIN' AND document_ref = $1",
    )
    .bind(FIRST_OF_THE_DAY)
    .fetch_one(&replay.desk.db.pool)
    .await
    .expect("the saved exchange");
    assert_eq!(status, "COMPLETED");
    let quote = quote.expect("the exchange is bound to the quote it was saved at (§4.6)");
    let (quote_status, accepted_by_the_exchange): (String, bool) = sqlx::query_as(
        "SELECT q.status, q.accepted_by_transaction_id = t.id \
         FROM quotes q JOIN transactions t ON t.quote_id = q.id \
         WHERE q.id = $1 AND t.document_ref = $2",
    )
    .bind(quote)
    .bind(FIRST_OF_THE_DAY)
    .fetch_one(&replay.desk.db.pool)
    .await
    .expect("the exchange's quote");
    assert_eq!(quote_status, "ACCEPTED");
    assert!(
        accepted_by_the_exchange,
        "the quote is accepted by this exchange"
    );
    assert_eq!(rows(&replay).await, (1, 1, 0), "one quote, one exchange");
    replay.finish().await;
}

// ── replay and CHANGED ───────────────────────────────────────────────────────

/// The same `submissionId` with the same payload answers the saved exchange and quotes nothing
/// again; with another payload it is `CHANGED` and leaves nothing (`save_transaction.rs:277-296`,
/// §4.6 "keyed by today's `submissionId`").
#[actix_web::test]
async fn a_repeated_submission_quotes_nothing_again() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    shift_holder_common::open_day(&replay.desk.db.pool, "MAIN").await;
    let payload = payload_of(&replay, LIVE_BUY_CASH);

    let first = save(&replay, &payload).await;
    assert_eq!(first.status, 200, "{:.300}", first.text);
    assert_eq!(rows(&replay).await, (1, 1, 0));

    let again = save(&replay, &payload).await;
    assert_eq!(again.status, 200, "{:.300}", again.text);
    assert_eq!(again.text, first.text, "the same answer, byte for byte");
    assert_eq!(rows(&replay).await, (1, 1, 0), "a repeat quotes nothing");

    let mut changed = payload.clone();
    changed["notes"] = json!("not the first notes");
    let answer = save(&replay, &changed).await;
    assert_eq!(
        refused(&answer, "the same submission, other notes"),
        CHANGED
    );
    assert_eq!(
        rows(&replay).await,
        (1, 1, 0),
        "a CHANGED save leaves nothing"
    );
    replay.finish().await;
}

// ── over the limit ───────────────────────────────────────────────────────────

/// Settling above the branch's approval limit is `approvalRequired` (§4.6: the adapter turns
/// `ConfirmOutcome::PendingApproval` into `Err(approvalRequired)`), and the shared transaction
/// rolls back the quote, the exchange and the approval request confirm wrote.
#[actix_web::test]
async fn a_compat_save_over_the_limit_is_approval_required_and_leaves_nothing() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    shift_holder_common::open_day(&replay.desk.db.pool, "MAIN").await;
    // #120 settles 3,220.00 THB; the limit is below it
    sqlx::query(
        "INSERT INTO branch_exchange_limits (organization_id, branch_id, approval_limit_thb, \
             updated_by, updated_at) \
         SELECT organization_id, id, 1000, 'test', now() FROM branches WHERE id = 'MAIN' \
         ON CONFLICT (organization_id, branch_id) \
         DO UPDATE SET approval_limit_thb = EXCLUDED.approval_limit_thb",
    )
    .execute(&replay.desk.db.pool)
    .await
    .expect("set MAIN's approval limit");

    let answer = save(&replay, &payload_of(&replay, LIVE_BUY_CASH)).await;
    let error = refused(&answer, "#120 over a limit of 1000 THB");
    assert!(
        error.starts_with("approvalRequired"),
        "refused as approvalRequired, got {error:?}"
    );
    assert_eq!(
        rows(&replay).await,
        NONE,
        "no quote, exchange or request is left"
    );
    replay.finish().await;
}

// ── refusals leave nothing ───────────────────────────────────────────────────

/// Stock the shift has not got is refused with today's sentence, and nothing is left: no quote,
/// no exchange, no request (§4.6).
#[actix_web::test]
async fn a_compat_save_refused_for_stock_leaves_nothing() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    shift_holder_common::open_day(&replay.desk.db.pool, "MAIN").await;
    let answer = save(&replay, &payload_of(&replay, NOT_ENOUGH_USD_AT)).await;
    assert_eq!(refused(&answer, "#114 SELL 750 USD"), NOT_ENOUGH_USD);
    assert_eq!(rows(&replay).await, NONE);
    replay.finish().await;
}

/// A custody refusal (the counter not open) leaves nothing (§4.6).
#[actix_web::test]
async fn a_compat_save_refused_by_custody_leaves_nothing() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    replay.desk.custody_on("MAIN").await;
    let answer = save(&replay, &payload_of(&replay, LIVE_BUY_CASH)).await;
    refused(&answer, "#120 with custody on and the counter closed");
    assert_eq!(rows(&replay).await, NONE);
    // CA-10 (#626 Q2): there is no custody-off switch any more, so the half that saved the same request with
    // custody off retired; the refusal and its empty write are the rule that stays.
    replay.finish().await;
}

/// A payment refusal (cash switched off for BUY) leaves nothing (§4.6).
#[actix_web::test]
async fn a_compat_save_refused_for_payment_leaves_nothing() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    shift_holder_common::open_day(&replay.desk.db.pool, "MAIN").await;
    let methods = json!([
        { "id": "Cash", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Credit Card", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Alipay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "PromptPay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Bank Transfer", "enabled": true, "buy": true, "sell": true, "account": "",
          "instruction": "If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน" },
    ]);
    let switched = replay
        .app
        .send(
            Call::action(
                ORIGIN,
                "/api/payment-methods",
                "save",
                json!({ "branchId": "MAIN", "version": 0, "methods": methods }),
            )
            .cookie(&replay.cookies["owner"]),
        )
        .await;
    assert_eq!(
        switched.status, 200,
        "cash off for BUY: {:.300}",
        switched.text
    );

    let answer = save(&replay, &payload_of(&replay, LIVE_BUY_CASH)).await;
    assert_eq!(refused(&answer, "#120, cash off for BUY"), UNAVAILABLE);
    assert_eq!(rows(&replay).await, NONE);
    replay.finish().await;
}

// ── #554 K4: the passport in the digest ──────────────────────────────────────

/// The digest of a submission takes the passport's `lookup_hash`, never the plain number
/// (design :755, #554 K4): the same submission with the passport spelled with spaces and in lower
/// case is the same exchange, answered as saved, not `CHANGED`. No stored row of the exchange or
/// its quote holds the plain number.
#[actix_web::test]
async fn the_passport_spelling_does_not_change_the_submission() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to_the_save(&fixture).await;
    shift_holder_common::open_day(&replay.desk.db.pool, "MAIN").await;
    let payload = payload_of(&replay, LIVE_BUY_CASH);
    assert_eq!(payload["passportNo"], json!(PASSPORT));

    let first = save(&replay, &payload).await;
    assert_eq!(first.status, 200, "{:.300}", first.text);

    let mut respelled = payload.clone();
    respelled["passportNo"] = json!(" p100 0001 ");
    let again = save(&replay, &respelled).await;
    assert_eq!(
        again.status, 200,
        "the passport re-spelled is the same submission: {:.300}",
        again.text
    );
    assert_eq!(again.text, first.text, "answered as saved");
    assert_eq!(rows(&replay).await, (1, 1, 0));

    let held: Vec<String> = sqlx::query_scalar(
        "SELECT row_to_json(t)::text FROM transactions t WHERE t.branch_id = 'MAIN' \
         UNION ALL SELECT row_to_json(q)::text FROM quotes q WHERE q.branch_id = 'MAIN'",
    )
    .fetch_all(&replay.desk.db.pool)
    .await
    .expect("the stored rows");
    assert_eq!(held.len(), 2, "the exchange and its quote");
    for row in &held {
        assert!(
            !row.to_ascii_uppercase().contains(PASSPORT),
            "a stored row holds the plain passport: {row}"
        );
    }
    replay.finish().await;
}
