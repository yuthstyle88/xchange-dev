//! Written before the implementation (row PM-S, design `docs/design/xchange-v3.md` §5.5.2).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The server checks again at the save: a **first LIVE** `saveTransaction` whose `method`
//! (`SCB Transfer` read as Bank Transfer) is off for its `type` in the branch is refused
//! `Payment method unavailable. Please contact staff.`, and nothing of it is kept. A repeat of
//! a saved submission and every DEMO save are not checked; what the engine refuses first is
//! still the engine's sentence (v2: `server.mjs:7058-7062` of the 10-06 bundle — the check runs
//! only when the engine saved something LIVE and the request was not an unchanged repeat).
//! Saved exchanges never change.
//!
//! The day of `rpc/engine_actions.json` is replayed through the routes (`rpc_common::Replay`)
//! up to the save under test; the owner switches channels at `POST /api/payment-methods`
//! (row PM-API) in between.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use compat_common::Fixture;
use rpc_common::{Answer, Call, Desk, RPC, Replay, STAFF_RPC};
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const FILE: &str = "rpc/engine_actions.json";
/// Who saves at the desk: the day's FRONT, FR1. v2's owner saved; the Blueprint's
/// `ExchangeTransact` is TELLER's only (OR-4b #767, design §4.2 C2), and B1 makes FRONT a TELLER.
const TELLER: &str = "FR1";
const UNAVAILABLE: &str = "Payment method unavailable. Please contact staff.";
/// A save at a rate other than the published one (decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): DEMO too).
const RATE_MISMATCH: &str = "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ";

/// `engine_actions.json`: #119 a LIVE BUY by cash refused by the engine (its notes are too
/// long), #120 a LIVE BUY by cash, #123 a LIVE SELL by PromptPay with a slip (the second
/// exchange of the day, `XC-20261006-000002`), #125 a DEMO BUY by cash.
const ENGINE_REFUSES: usize = 119;
const LIVE_BUY_CASH: usize = 120;
const LIVE_SELL_PROMPTPAY: usize = 123;
const DEMO_BUY_CASH: usize = 125;
const SECOND_OF_THE_DAY: &str = "XC-20261006-000002";

/// The first bytes of a PNG: enough for the upload route.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
];

/// v2's defaults (`payment-settings.mjs:3` of the 10-06 bundle).
fn defaults() -> Value {
    json!([
        { "id": "Cash", "enabled": true, "buy": true, "sell": true, "account": "", "instruction": "" },
        { "id": "Credit Card", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Alipay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "PromptPay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Bank Transfer", "enabled": true, "buy": true, "sell": true, "account": "",
          "instruction": "If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน" },
    ])
}

fn with(id: &str, change: Value) -> Value {
    let mut rows = defaults();
    for row in rows.as_array_mut().expect("five rows") {
        if row["id"] == json!(id) {
            for (k, v) in change.as_object().expect("changes") {
                row[k] = v.clone();
            }
        }
    }
    rows
}

/// The day replayed through the routes up to and including step `last`.
async fn day_up_to(fixture: &Fixture, last: usize) -> Replay<'_> {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
    for n in 1..=last {
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

async fn owner_switches(replay: &Replay<'_>, version: i64, methods: Value) {
    let answer = replay
        .app
        .send(
            Call::action(
                ORIGIN,
                "/api/payment-methods",
                "save",
                json!({ "branchId": "MAIN", "version": version, "methods": methods }),
            )
            .cookie(&replay.cookies["owner"]),
        )
        .await;
    assert_eq!(
        answer.status, 200,
        "the owner switches channels: {:.300}",
        answer.text
    );
}

async fn save(replay: &Replay<'_>, who: &str, route: &str, payload: &Value) -> Answer {
    // OR-4b (C2): a save attaches its saver's own files; the teller uploads the owner's again
    let payload = replay.uploads_as(who, payload).await;
    replay
        .app
        .send(Call::action(ORIGIN, route, "saveTransaction", payload).cookie(&replay.cookies[who]))
        .await
}

/// What a refused save must leave as it was: the exchanges, the stock and the customers of
/// `MAIN` as the owner's snapshot shows them.
async fn kept(replay: &Replay<'_>) -> Value {
    let answer = replay
        .app
        .send(
            Call::action(ORIGIN, RPC, "adminSnapshot", json!({ "branchId": "MAIN" }))
                .cookie(&replay.cookies["owner"]),
        )
        .await;
    assert_eq!(answer.status, 200, "the snapshot: {:.300}", answer.text);
    let snapshot = answer.json();
    json!({
        "transactions": snapshot["transactions"],
        "stock": snapshot["stock"],
        "customers": snapshot["customers"],
    })
}

fn documents(kept: &Value) -> Vec<String> {
    kept["transactions"]
        .as_array()
        .expect("the exchanges")
        .iter()
        .map(|t| t["id"].as_str().expect("a document number").to_string())
        .collect()
}

#[actix_web::test]
async fn a_first_live_save_on_a_channel_switched_off_is_refused_and_nothing_is_kept() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_SELL_PROMPTPAY - 1).await;
    owner_switches(&replay, 0, with("PromptPay", json!({ "enabled": false }))).await;
    let before = kept(&replay).await;

    let payload = payload_of(&replay, LIVE_SELL_PROMPTPAY);
    save(&replay, TELLER, RPC, &payload).await.assert_refused(
        400,
        UNAVAILABLE,
        "LIVE SELL by PromptPay, PromptPay off",
    );
    let after = kept(&replay).await;
    assert_eq!(after, before, "the refused save left nothing behind");
    assert!(!documents(&after).contains(&SECOND_OF_THE_DAY.to_string()));

    // on again: the very same request is saved, under the number v2 gave it
    owner_switches(&replay, 1, defaults()).await;
    let saved = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    assert_eq!(saved.json()["documentRef"], json!(SECOND_OF_THE_DAY));
    replay.finish().await;
}

/// Only the save's own direction is checked: Cash off for BUY refuses a cash BUY; Cash off for
/// SELL alone lets the same cash BUY through.
#[actix_web::test]
async fn the_direction_of_the_save_is_the_one_checked() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY_CASH - 1).await;
    owner_switches(&replay, 0, with("Cash", json!({ "buy": false }))).await;
    let payload = payload_of(&replay, LIVE_BUY_CASH);
    save(&replay, TELLER, RPC, &payload).await.assert_refused(
        400,
        UNAVAILABLE,
        "LIVE BUY by cash, cash pays no baht out",
    );

    // Cash still takes baht in
    owner_switches(&replay, 1, with("Cash", json!({ "sell": false }))).await;
    let saved = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    replay.finish().await;
}

/// A repeat of a submission already saved answers the saved exchange, whatever was switched off
/// since: saved exchanges never change.
#[actix_web::test]
async fn a_repeat_of_a_saved_submission_is_not_checked() {
    let fixture = Fixture::load(FILE);
    // OR-4b (#767, C2): the teller makes the save v2's owner made at this step (a save attaches its
    // saver's own files, so only the saver repeats it), then repeats it
    let replay = day_up_to(&fixture, LIVE_SELL_PROMPTPAY - 1).await;
    let payload = payload_of(&replay, LIVE_SELL_PROMPTPAY);
    let first = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(first.status, 200, "{:.300}", first.text);
    let before = kept(&replay).await;
    assert!(documents(&before).contains(&SECOND_OF_THE_DAY.to_string()));

    owner_switches(&replay, 0, with("PromptPay", json!({ "enabled": false }))).await;
    let again = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(again.status, 200, "{:.300}", again.text);
    assert_eq!(again.json()["documentRef"], json!(SECOND_OF_THE_DAY));
    assert_eq!(kept(&replay).await, before, "nothing changed");
    replay.finish().await;
}

/// A DEMO save is never checked (v2 checks `mode === "LIVE"` only).
#[actix_web::test]
async fn a_demo_save_is_not_checked() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, DEMO_BUY_CASH - 1).await;
    owner_switches(&replay, 0, with("Cash", json!({ "buy": false }))).await;
    let mut payload = payload_of(&replay, DEMO_BUY_CASH);
    assert_eq!(payload["mode"], json!("DEMO"));
    // Re-expected for TX-6 by the test-writer, decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): a DEMO save is quoted like a LIVE one,
    // so #125's rate 1 is refused; the save is sent at the published 32.2 and is still not checked.
    save(&replay, TELLER, RPC, &payload).await.assert_refused(
        400,
        RATE_MISMATCH,
        "#125 at rate 1 (decision A2)",
    );
    payload["rate"] = json!(32.2);
    let saved = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    assert_eq!(saved.json()["mode"], json!("DEMO"));
    replay.finish().await;
}

/// The old name `SCB Transfer` is the Bank Transfer channel.
#[actix_web::test]
async fn scb_transfer_is_read_as_bank_transfer() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_SELL_PROMPTPAY - 1).await;
    let mut payload = payload_of(&replay, LIVE_SELL_PROMPTPAY);
    payload["method"] = json!("SCB Transfer");
    payload["account"] = json!("SCB 000");

    owner_switches(&replay, 0, with("Bank Transfer", json!({ "sell": false }))).await;
    save(&replay, TELLER, RPC, &payload).await.assert_refused(
        400,
        UNAVAILABLE,
        "SCB Transfer, Bank Transfer takes no baht in",
    );
    let mut bank = payload.clone();
    bank["method"] = json!("Bank Transfer");
    save(&replay, TELLER, RPC, &bank).await.assert_refused(
        400,
        UNAVAILABLE,
        "Bank Transfer, off for SELL",
    );

    owner_switches(&replay, 1, with("PromptPay", json!({ "enabled": false }))).await;
    let saved = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    replay.finish().await;
}

/// The payment check comes after the engine's own: a save the engine refuses keeps the engine's
/// sentence, whatever is switched off.
#[actix_web::test]
async fn the_engines_own_refusals_come_first() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, ENGINE_REFUSES - 1).await;
    let recorded = fixture.step(ENGINE_REFUSES);
    let sentence = recorded.error().expect("v2 refused it").to_string();
    assert_ne!(sentence, UNAVAILABLE);
    owner_switches(&replay, 0, with("Cash", json!({ "buy": false }))).await;
    let payload = payload_of(&replay, ENGINE_REFUSES);
    let answer = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(answer.status, 400, "{:.300}", answer.text);
    assert_eq!(answer.error(), sentence);
    replay.finish().await;
}

/// The staff route saves through the same action: a Front's LIVE save is checked too.
#[actix_web::test]
async fn the_staff_route_is_checked_too() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_SELL_PROMPTPAY - 1).await;
    let mut keys = Vec::new();
    for what in ["passport", "slip"] {
        let uploaded = replay
            .app
            .send(
                Call::post(ORIGIN, "/api/staff/upload", PNG)
                    .header("content-type", "image/png")
                    .cookie(&replay.cookies["FR1"]),
            )
            .await;
        assert_eq!(uploaded.status, 200, "the {what}: {:.300}", uploaded.text);
        keys.push(uploaded.json()["uploadKey"].clone());
    }
    let mut payload = payload_of(&replay, LIVE_SELL_PROMPTPAY);
    payload["passport"] = json!({ "uploadKey": keys[0] });
    payload["slip"] = json!({ "uploadKey": keys[1] });

    owner_switches(&replay, 0, with("PromptPay", json!({ "enabled": false }))).await;
    save(&replay, "FR1", STAFF_RPC, &payload)
        .await
        .assert_refused(400, UNAVAILABLE, "a Front's LIVE SELL by PromptPay");
    owner_switches(&replay, 1, defaults()).await;
    let saved = save(&replay, "FR1", STAFF_RPC, &payload).await;
    assert_eq!(saved.status, 200, "{:.300}", saved.text);
    replay.finish().await;
}
