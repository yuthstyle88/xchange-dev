//! Written before the implementation (row T15-h, ticket #414; design `docs/design/xchange-v3.md`
//! §5.6.3 *Using a booking at the desk*, §5.6.5 *The save seam*, §5.6.8).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! A desk opened from a booking sends `saveTransaction` with `onlineBookingId`, `booking` = the
//! id, the booked type, currency, amount and rate, and `onlineBookingCustomerConfirmed: true`.
//! Inside the save's branch transaction, after the form is judged and before the rate check, the
//! booking is checked (v2: `validateBookingForExchange` and the member check of the 10-06
//! bundle, `src/online/lib/booking-rules.mjs` and `src/online/lib/ledger.ts` of
//! `108-Plaza/xchange-v2-source@4d0bdbb`):
//!
//! 1. no booking with that id in the save's branch → `ไม่พบการจองของสาขานี้`
//! 2. stored `USED` by another submission → `การจองนี้ถูกใช้ทำรายการแล้ว` (by this one: a repeat)
//! 3. not `USED`, and not `CONFIRMED` or `expiresAt` ≤ now → `การจองยังไม่ยืนยัน หมดอายุ หรือถูกยกเลิกแล้ว`
//! 4. not LIVE, or type, currency, amount, rate, `booking` or the customer tick differ →
//!    `ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า`
//! 5. a member's booking for another customer → `ลูกค้าไม่ตรงกับ Member ที่จอง`
//!
//! A booking that passes skips the counter-rate check (the booked rate stands), and a first save
//! marks it `USED` with its submission and document number. A refused save keeps nothing.
//!
//! The day of `rpc/engine_actions.json` is replayed through the routes (`rpc_common::Replay`) up
//! to the save under test. Bookings and members are written into their tables (T15-b) directly:
//! no route makes them before T15-f, and this file depends on no route of T15-f.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use compat_common::Fixture;
use rpc_common::{Answer, Call, Desk, RPC, Replay, STAFF_RPC};
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const FILE: &str = "rpc/engine_actions.json";

const NOT_FOUND_HERE: &str = "ไม่พบการจองของสาขานี้";
const USED_BY_ANOTHER: &str = "การจองนี้ถูกใช้ทำรายการแล้ว";
const NOT_USABLE: &str = "การจองยังไม่ยืนยัน หมดอายุ หรือถูกยกเลิกแล้ว";
const FORM_MISMATCH: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";
const MEMBER_MISMATCH: &str = "ลูกค้าไม่ตรงกับ Member ที่จอง";
/// `save_transaction.rs:36`.
const RATE_MISMATCH: &str = "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ";
/// `save_transaction.rs:35`.
const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";

/// `engine_actions.json`: the counter rate of USD is buy 32.2, sell 32.7 (#28). #119 a LIVE BUY
/// the engine refuses (its notes are too long); #120 a LIVE BUY of USD 100 at 32.2 by cash for
/// `CX-00000001` at 07:00:00.000Z; #123 a LIVE SELL of USD 20 at 32.7 by PromptPay with a slip;
/// #125 a DEMO BUY of USD 100 at 1.
const ENGINE_REFUSES: usize = 119;
const LIVE_BUY: usize = 120;
const LIVE_SELL: usize = 123;
const DEMO_BUY: usize = 125;
/// The time of #120 and #123's day: a booking open until then is expired at the save of #120.
const SAVE_AT_120: &str = "2026-10-06T07:00:00.000Z";
const OPEN_UNTIL: &str = "2026-10-06T07:30:00.000Z";
/// Customer 3 of the day (#80), another LIVE customer of `MAIN`.
const OTHER_PASSPORT: &str = "P1000003";

/// Submission ids no step of the day uses.
const FRESH_A: &str = "00000000-0000-4000-8000-0000000bf0a1";
/// Bookings raced for in `two_saves_at_once_use_the_booking_once`.
const RACES: usize = 5;

/// The first bytes of a PNG: enough for the upload route.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
];

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

/// `payload` as the desk opened from booking `id` sends it: the booking named twice and the
/// customer ticked.
fn claiming(payload: &Value, id: &str) -> Value {
    let mut out = payload.clone();
    out["onlineBookingId"] = json!(id);
    out["booking"] = json!(id);
    out["onlineBookingCustomerConfirmed"] = json!(true);
    out
}

/// A booking row of T15-b's table. `fields` overrides the defaults: a CONFIRMED BUY of USD 100
/// at 32.2 in `MAIN`, open until [`OPEN_UNTIL`], no member.
async fn book(replay: &Replay<'_>, id: &str, fields: Value) {
    let mut row = json!({
        "branch": "MAIN",
        "type": "BUY",
        "code": "USD",
        "amount": "100",
        "rate": "32.2",
        "status": "CONFIRMED",
        "expiresAt": OPEN_UNTIL,
        "memberId": null,
    });
    for (k, v) in fields.as_object().expect("the fields of a booking") {
        assert!(row.get(k).is_some(), "no booking field {k}");
        row[k] = v.clone();
    }
    let text = |k: &str| row[k].as_str().map(str::to_owned);
    sqlx::query(
        "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
         type, code, amount, rate, status, created_at, created_by, expires_at) \
         VALUES ($1, $2, $1, $1, 'Booked Customer', '0812345678', $3, $4, $5, $6::numeric, \
         $7::numeric, $8, '2026-10-06T06:00:00.000Z'::timestamptz, 'owner', $9::timestamptz)",
    )
    .bind(id)
    .bind(text("branch"))
    .bind(text("memberId"))
    .bind(text("type"))
    .bind(text("code"))
    .bind(text("amount"))
    .bind(text("rate"))
    .bind(text("status"))
    .bind(text("expiresAt"))
    .execute(&replay.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("book {id}: {e}"));
}

/// The stored status of booking `id`, what used it and the document it became.
async fn booking_row(replay: &Replay<'_>, id: &str) -> Value {
    let (status, submission, document): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status, submission_id, transaction_ref FROM bookings WHERE id = $1")
            .bind(id)
            .fetch_one(&replay.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("read booking {id}: {e}"));
    json!({ "status": status, "submission_id": submission, "transaction_ref": document })
}

fn untouched() -> Value {
    json!({ "status": "CONFIRMED", "submission_id": null, "transaction_ref": null })
}

fn used_by(submission: &str, document: &str) -> Value {
    json!({ "status": "USED", "submission_id": submission, "transaction_ref": document })
}

/// A member of `MAIN` for customer `customer`; answers the member's id.
async fn member_of(replay: &Replay<'_>, id: &str, customer: &str) -> String {
    sqlx::query(
        "INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, \
         verified_by) VALUES ($1, 'MAIN', $2, 'Member', '0812345678', $1, \
         '2026-10-06T06:00:00.000Z'::timestamptz, 'owner')",
    )
    .bind(id)
    .bind(customer)
    .execute(&replay.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("member {id}: {e}"));
    id.to_string()
}

/// The id v3 gave the LIVE customer with this passport.
async fn customer_with(replay: &Replay<'_>, passport: &str) -> String {
    // CI-4 (#651): one customer per company with an encrypted passport (#554 K1, K2).
    snapshot_common::live_customer_with_passport(&replay.desk.db.pool, passport)
        .await
        .unwrap_or_else(|| panic!("the customer with {passport}"))
}

async fn save(replay: &Replay<'_>, who: &str, route: &str, payload: &Value) -> Answer {
    replay
        .app
        .send(
            Call::action(ORIGIN, route, "saveTransaction", payload.clone())
                .cookie(&replay.cookies[who]),
        )
        .await
}

fn saved_ref(answer: &Answer, label: &str) -> String {
    assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
    answer.json()["documentRef"]
        .as_str()
        .unwrap_or_else(|| panic!("{label}: a document number: {:.300}", answer.text))
        .to_string()
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

/// The exchange `document` as the owner's snapshot prints it.
fn exchange<'a>(kept: &'a Value, document: &str) -> &'a Value {
    kept["transactions"]
        .as_array()
        .expect("the exchanges")
        .iter()
        .find(|t| t["id"] == json!(document))
        .unwrap_or_else(|| panic!("{document} is in the snapshot"))
}

/// A refusal that leaves the exchanges, the stock, the customers and the booking as they were.
async fn refused_and_kept(
    replay: &Replay<'_>,
    who: &str,
    route: &str,
    payload: &Value,
    sentence: &str,
    booking: &str,
    label: &str,
) {
    let before = kept(replay).await;
    let row = booking_row(replay, booking).await;
    save(replay, who, route, payload)
        .await
        .assert_refused(400, sentence, label);
    assert_eq!(
        kept(replay).await,
        before,
        "{label}: nothing of it was kept"
    );
    assert_eq!(
        booking_row(replay, booking).await,
        row,
        "{label}: the booking is as it was"
    );
}

/// The booked rate stands: a booking at 32.1 is saved at 32.1 while the counter buys at 32.2, and
/// the booking becomes USED by this submission and its document.
#[actix_web::test]
async fn booked_rate_skips_counter_check() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-rate", json!({ "rate": "32.1" })).await;
    let mut payload = claiming(&payload_of(&replay, LIVE_BUY), "BK-rate");
    payload["rate"] = json!(32.1);

    let answer = save(&replay, "owner", RPC, &payload).await;
    let document = saved_ref(&answer, "a booking at 32.1, the counter at 32.2");
    let after = kept(&replay).await;
    assert_eq!(
        exchange(&after, &document)["rate"].to_string(),
        "32.1",
        "saved at the booked rate"
    );
    let submission = payload["submissionId"].as_str().expect("a submission");
    assert_eq!(
        booking_row(&replay, "BK-rate").await,
        used_by(submission, &document)
    );
    replay.finish().await;
}

/// A save that names no booking is judged as before: a rate that is not the counter rate is
/// refused, whatever the form's `booking` says and however the customer was ticked. An empty
/// `onlineBookingId` names no booking (v2 checks it only when it is truthy).
#[actix_web::test]
async fn no_claim_keeps_rate_check() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-rate", json!({ "rate": "32.1" })).await;
    let claimed = claiming(&payload_of(&replay, LIVE_BUY), "BK-rate");

    let mut absent = claimed.clone();
    absent["rate"] = json!(32.1);
    absent
        .as_object_mut()
        .expect("a payload")
        .remove("onlineBookingId");
    let mut empty = absent.clone();
    empty["onlineBookingId"] = json!("");
    let mut null = absent.clone();
    null["onlineBookingId"] = Value::Null;

    for (payload, label) in [
        (&absent, "no onlineBookingId"),
        (&empty, "onlineBookingId empty"),
        (&null, "onlineBookingId null"),
    ] {
        refused_and_kept(
            &replay,
            "owner",
            RPC,
            payload,
            RATE_MISMATCH,
            "BK-rate",
            label,
        )
        .await;
    }
    assert_eq!(booking_row(&replay, "BK-rate").await, untouched());
    replay.finish().await;
}

/// One booking, two submissions: the first saves and uses it, the second is refused and keeps
/// nothing.
#[actix_web::test]
async fn second_submission_refused() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-once", json!({})).await;
    let first = claiming(&payload_of(&replay, LIVE_BUY), "BK-once");
    let document = saved_ref(&save(&replay, "owner", RPC, &first).await, "the first save");
    let submission = first["submissionId"].as_str().expect("a submission");
    assert_eq!(
        booking_row(&replay, "BK-once").await,
        used_by(submission, &document)
    );

    let mut second = first.clone();
    second["submissionId"] = json!(FRESH_A);
    second["notes"] = json!("another exchange");
    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &second,
        USED_BY_ANOTHER,
        "BK-once",
        "a second submission with the same booking",
    )
    .await;
    replay.finish().await;
}

/// The same save sent again answers the stored exchange; the booking stays USED by it and
/// nothing is written again.
#[actix_web::test]
async fn repeat_passes() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-repeat", json!({ "rate": "32.1" })).await;
    let mut payload = claiming(&payload_of(&replay, LIVE_BUY), "BK-repeat");
    payload["rate"] = json!(32.1);
    let document = saved_ref(
        &save(&replay, "owner", RPC, &payload).await,
        "the first save",
    );
    let submission = payload["submissionId"].as_str().expect("a submission");
    let before = kept(&replay).await;

    let again = save(&replay, "owner", RPC, &payload).await;
    assert_eq!(
        saved_ref(&again, "the repeat"),
        document,
        "the stored answer"
    );
    assert_eq!(kept(&replay).await, before, "nothing written again");
    assert_eq!(
        booking_row(&replay, "BK-repeat").await,
        used_by(submission, &document)
    );
    replay.finish().await;
}

/// A member's booking is for that member's customer only.
#[actix_web::test]
async fn member_customer_must_match() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);
    let customer = payload["customerId"]
        .as_str()
        .expect("the customer")
        .to_string();
    let other = customer_with(&replay, OTHER_PASSPORT).await;
    assert_ne!(other, customer);
    let theirs = member_of(&replay, "MB-other", &other).await;
    let ours = member_of(&replay, "MB-ours", &customer).await;
    book(&replay, "BK-theirs", json!({ "memberId": theirs })).await;
    book(&replay, "BK-ours", json!({ "memberId": ours })).await;

    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &claiming(&payload, "BK-theirs"),
        MEMBER_MISMATCH,
        "BK-theirs",
        "another member's booking",
    )
    .await;

    let mine = claiming(&payload, "BK-ours");
    let document = saved_ref(
        &save(&replay, "owner", RPC, &mine).await,
        "the member's own booking",
    );
    let submission = mine["submissionId"].as_str().expect("a submission");
    assert_eq!(
        booking_row(&replay, "BK-ours").await,
        used_by(submission, &document)
    );
    assert_eq!(booking_row(&replay, "BK-theirs").await, untouched());
    replay.finish().await;
}

/// A booking cancelled, or expired, after the desk read it is refused at the save: cancelled,
/// open until the very instant of the save (expiry is inclusive), open until before it, and a
/// member's booking still waiting for its expiry (PENDING).
#[actix_web::test]
async fn cancelled_or_expired_refused() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);
    let customer = payload["customerId"]
        .as_str()
        .expect("the customer")
        .to_string();
    let ours = member_of(&replay, "MB-ours", &customer).await;
    book(&replay, "BK-cancelled", json!({})).await;
    book(&replay, "BK-at-the-instant", json!({})).await;
    book(&replay, "BK-before", json!({})).await;
    book(
        &replay,
        "BK-pending",
        json!({ "status": "PENDING", "expiresAt": null, "memberId": ours }),
    )
    .await;
    // what the desk read with `get` changed before the save
    for (sql, id) in [
        (
            "UPDATE bookings SET status = 'CANCELLED' WHERE id = $1",
            "BK-cancelled",
        ),
        (
            "UPDATE bookings SET expires_at = '2026-10-06T07:00:00.000Z'::timestamptz WHERE id = $1",
            "BK-at-the-instant",
        ),
        (
            "UPDATE bookings SET expires_at = '2026-10-06T06:59:59.999Z'::timestamptz WHERE id = $1",
            "BK-before",
        ),
    ] {
        sqlx::query(sql)
            .bind(id)
            .execute(&replay.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("change {id}: {e}"));
    }
    assert_eq!(fixture.step(LIVE_BUY).at, SAVE_AT_120);

    for id in [
        "BK-cancelled",
        "BK-at-the-instant",
        "BK-before",
        "BK-pending",
    ] {
        refused_and_kept(
            &replay,
            "owner",
            RPC,
            &claiming(&payload, id),
            NOT_USABLE,
            id,
            id,
        )
        .await;
    }
    replay.finish().await;
}

/// A booking is found in the save's own branch only.
#[actix_web::test]
async fn unknown_or_other_branch_booking_refused() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    sqlx::query("INSERT INTO branches (id, name) VALUES ('B2', 'Branch Two')")
        .execute(&replay.desk.db.pool)
        .await
        .expect("a second branch");
    book(&replay, "BK-elsewhere", json!({ "branch": "B2" })).await;
    book(&replay, "BK-here", json!({})).await;
    let payload = payload_of(&replay, LIVE_BUY);

    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &claiming(&payload, "BK-elsewhere"),
        NOT_FOUND_HERE,
        "BK-elsewhere",
        "a booking of another branch",
    )
    .await;
    let mut nowhere = claiming(&payload, "BK-here");
    nowhere["onlineBookingId"] = json!("BK-nowhere");
    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &nowhere,
        NOT_FOUND_HERE,
        "BK-here",
        "no booking with that id",
    )
    .await;
    replay.finish().await;
}

/// The form must be the booking's: LIVE, its type, currency, amount and rate, `booking` naming
/// it, and the customer ticked with JSON `true`. A form at the counter rate does not pass for a
/// booking at another rate.
#[actix_web::test]
async fn form_must_match_the_booking() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-match", json!({ "rate": "32.1" })).await;
    let mut good = claiming(&payload_of(&replay, LIVE_BUY), "BK-match");
    good["rate"] = json!(32.1);

    let variant = |key: &str, value: Value| {
        let mut p = good.clone();
        p[key] = value;
        p
    };
    let mut unticked = good.clone();
    unticked
        .as_object_mut()
        .expect("a payload")
        .remove("onlineBookingCustomerConfirmed");
    let cases = [
        (
            variant("rate", json!(32.2)),
            "the counter rate, not the booked one",
        ),
        (variant("amount", json!(99)), "another amount"),
        (variant("currency", json!("EUR")), "another currency"),
        (variant("type", json!("SELL")), "another type"),
        (variant("booking", json!("WALK-IN")), "booking not the id"),
        (
            variant("onlineBookingCustomerConfirmed", json!("true")),
            "the tick as text",
        ),
        (
            variant("onlineBookingCustomerConfirmed", json!(false)),
            "the tick false",
        ),
        (unticked, "no tick"),
    ];
    for (payload, label) in &cases {
        refused_and_kept(
            &replay,
            "owner",
            RPC,
            payload,
            FORM_MISMATCH,
            "BK-match",
            label,
        )
        .await;
    }

    let document = saved_ref(
        &save(&replay, "owner", RPC, &good).await,
        "the matching form",
    );
    let submission = good["submissionId"].as_str().expect("a submission");
    assert_eq!(
        booking_row(&replay, "BK-match").await,
        used_by(submission, &document)
    );
    replay.finish().await;
}

/// A DEMO save cannot use a booking (v2: `p.mode !== 'LIVE'`).
#[actix_web::test]
async fn demo_save_cannot_use_a_booking() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, DEMO_BUY - 1).await;
    book(&replay, "BK-demo", json!({})).await;
    let mut payload = claiming(&payload_of(&replay, DEMO_BUY), "BK-demo");
    assert_eq!(payload["mode"], json!("DEMO"));
    payload["rate"] = json!(32.2);
    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &payload,
        FORM_MISMATCH,
        "BK-demo",
        "a DEMO save naming a booking",
    )
    .await;
    replay.finish().await;
}

/// The refusals come in v2's order: found (1), used (2), usable (3), the form (4), the member (5).
#[actix_web::test]
async fn refusal_order() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);
    let other = customer_with(&replay, OTHER_PASSPORT).await;
    let theirs = member_of(&replay, "MB-other", &other).await;
    book(&replay, "BK-cancelled", json!({ "status": "CANCELLED" })).await;
    book(&replay, "BK-theirs", json!({ "memberId": theirs })).await;
    book(&replay, "BK-once", json!({})).await;
    let first = claiming(&payload, "BK-once");
    saved_ref(&save(&replay, "owner", RPC, &first).await, "BK-once used");

    let wrong_amount = |id: &str| {
        let mut p = claiming(&payload, id);
        p["submissionId"] = json!(FRESH_A);
        p["amount"] = json!(99);
        p
    };
    for (id, sentence, label) in [
        (
            "BK-once",
            USED_BY_ANOTHER,
            "used by another before the form (2 before 4)",
        ),
        (
            "BK-cancelled",
            NOT_USABLE,
            "cancelled before the form (3 before 4)",
        ),
        (
            "BK-theirs",
            FORM_MISMATCH,
            "the form before the member (4 before 5)",
        ),
    ] {
        refused_and_kept(
            &replay,
            "owner",
            RPC,
            &wrong_amount(id),
            sentence,
            id,
            label,
        )
        .await;
    }
    replay.finish().await;
}

/// A save refused after the booking passed its check keeps the booking as it was: here the
/// submission was already saved with other data (`CHANGED`). The booking is still there for the
/// next save.
#[actix_web::test]
async fn a_save_refused_after_the_check_keeps_the_booking() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-kept", json!({})).await;
    let walk_in = payload_of(&replay, LIVE_BUY);
    saved_ref(
        &save(&replay, "owner", RPC, &walk_in).await,
        "the walk-in save",
    );

    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &claiming(&walk_in, "BK-kept"),
        CHANGED,
        "BK-kept",
        "the saved submission sent again with the booking",
    )
    .await;
    assert_eq!(booking_row(&replay, "BK-kept").await, untouched());

    let mut fresh = claiming(&walk_in, "BK-kept");
    fresh["submissionId"] = json!(FRESH_A);
    let document = saved_ref(
        &save(&replay, "owner", RPC, &fresh).await,
        "a new submission with the booking",
    );
    assert_eq!(
        booking_row(&replay, "BK-kept").await,
        used_by(FRESH_A, &document)
    );
    replay.finish().await;
}

/// Two desks save with one booking at the same moment — the owner on `/api/rpc` and a Front on
/// `/api/staff/rpc`, two sessions, so the two requests really run side by side: exactly one
/// exchange is saved, the other is refused with v2's sentence (not a server error), and the
/// booking is USED by the one saved. Done for [`RACES`] bookings in turn, so that a check made
/// outside the save's branch transaction has more than one chance to be caught.
#[actix_web::test]
async fn two_saves_at_once_use_the_booking_once() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let uploaded = replay
        .app
        .send(
            Call::post(ORIGIN, "/api/staff/upload", PNG)
                .header("content-type", "image/png")
                .cookie(&replay.cookies["FR1"]),
        )
        .await;
    assert_eq!(
        uploaded.status, 200,
        "the Front's passport: {:.300}",
        uploaded.text
    );
    let front_passport = json!({ "uploadKey": uploaded.json()["uploadKey"].clone() });
    let walk_in = payload_of(&replay, LIVE_BUY);

    for round in 0..RACES {
        let id = format!("BK-race-{round}");
        book(&replay, &id, json!({})).await;
        let base = claiming(&walk_in, &id);
        let (sub_a, sub_b) = (
            format!("00000000-0000-4000-8000-0000000bfa{round:02}"),
            format!("00000000-0000-4000-8000-0000000bfb{round:02}"),
        );
        let mut a = base.clone();
        a["submissionId"] = json!(sub_a);
        let mut b = base.clone();
        b["passport"] = front_passport.clone();
        b["submissionId"] = json!(sub_b);
        b["notes"] = json!("the other desk");
        let before = kept(&replay).await["transactions"]
            .as_array()
            .expect("the exchanges")
            .len();

        let (first, second) = tokio::join!(
            save(&replay, "owner", RPC, &a),
            save(&replay, "FR1", STAFF_RPC, &b)
        );
        let (won, lost, winner) = match (first.status, second.status) {
            (200, _) => (first, second, sub_a),
            (_, 200) => (second, first, sub_b),
            _ => panic!(
                "{id}: one of the two is saved: {:.300} / {:.300}",
                first.text, second.text
            ),
        };
        lost.assert_refused(400, USED_BY_ANOTHER, &format!("{id}: the other desk"));
        let document = saved_ref(&won, &format!("{id}: the desk that won"));
        assert_eq!(
            booking_row(&replay, &id).await,
            used_by(&winner, &document),
            "{id}"
        );
        let after = kept(&replay).await["transactions"]
            .as_array()
            .expect("the exchanges")
            .len();
        assert_eq!(after, before + 1, "{id}: exactly one exchange was saved");
    }
    replay.finish().await;
}

/// The staff route saves through the same action: a Front's save from a booking at a rate the
/// counter no longer sells at is saved, and uses the booking.
#[actix_web::test]
async fn the_staff_route_uses_the_booking_too() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_SELL - 1).await;
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
    book(
        &replay,
        "BK-staff",
        json!({ "type": "SELL", "amount": "20", "rate": "32.65" }),
    )
    .await;
    let mut payload = claiming(&payload_of(&replay, LIVE_SELL), "BK-staff");
    payload["passport"] = json!({ "uploadKey": keys[0] });
    payload["slip"] = json!({ "uploadKey": keys[1] });
    payload["rate"] = json!(32.65);

    let mut no_claim = payload.clone();
    no_claim
        .as_object_mut()
        .expect("a payload")
        .remove("onlineBookingId");
    refused_and_kept(
        &replay,
        "FR1",
        STAFF_RPC,
        &no_claim,
        RATE_MISMATCH,
        "BK-staff",
        "a Front's SELL at 32.65 with no booking",
    )
    .await;

    let document = saved_ref(
        &save(&replay, "FR1", STAFF_RPC, &payload).await,
        "a Front's SELL from the booking",
    );
    assert_eq!(
        exchange(&kept(&replay).await, &document)["rate"].to_string(),
        "32.65",
        "saved at the booked rate"
    );
    let submission = payload["submissionId"].as_str().expect("a submission");
    assert_eq!(
        booking_row(&replay, "BK-staff").await,
        used_by(submission, &document)
    );
    replay.finish().await;
}

/// The booking is checked after the form is judged: a bad form with a bad booking answers the
/// form's refusal (§5.6.7, row *A bad form with a bad booking*: v2 checked the booking first).
#[actix_web::test]
async fn a_bad_form_with_a_bad_booking_answers_the_form() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, ENGINE_REFUSES - 1).await;
    book(&replay, "BK-here", json!({})).await;
    let recorded = fixture.step(ENGINE_REFUSES);
    let sentence = recorded.error().expect("v2 refused it").to_string();
    let mut payload = claiming(&payload_of(&replay, ENGINE_REFUSES), "BK-here");
    payload["onlineBookingId"] = json!("BK-nowhere");
    refused_and_kept(
        &replay,
        "owner",
        RPC,
        &payload,
        &sentence,
        "BK-here",
        "a form the engine refuses, naming no booking of the branch",
    )
    .await;
    replay.finish().await;
}
