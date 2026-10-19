//! Written before the implementation (row T15-h, ticket #414; design `docs/design/xchange-v3.md`
//! §5.6.3 *Using a booking at the desk*, §5.6.5 *The save seam*, §5.6.8).
//! Re-pointed to Blueprint `bookings` for BK-7 (#1405, rulings R1-R6); Blueprint bookings completion
//! for BK-7 is also tested in `api/tests/booking_completion_test.rs`.
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

use chrono::Utc;
use compat_common::Fixture;
use rpc_common::{Answer, Call, Desk, RPC, Replay, STAFF_RPC};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use snapshot_common::time;
use uuid::Uuid;

const ORIGIN: &str = "http://desk.test";
const FILE: &str = "rpc/engine_actions.json";
/// Who saves at the desk: the day's FRONT, FR1. v2's owner saved; the Blueprint's
/// `ExchangeTransact` is TELLER's only (OR-4b #767, design §4.2 C2), and B1 makes FRONT a TELLER.
const TELLER: &str = "FR1";

const NOT_FOUND_HERE: &str = "bookingNotUsable";
const USED_BY_ANOTHER: &str = "bookingNotUsable";
const NOT_USABLE: &str = "bookingNotUsable";
const NOT_THIS_CUSTOMER: &str = "bookingNotThisCustomer";
/// `save_transaction.rs:36`.
const RATE_MISMATCH: &str = "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ";
/// `save_transaction.rs:35`.
const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";

/// `engine_actions.json`: the counter rate of USD is buy 32.2, sell 32.7 (#28). #119 a LIVE BUY
/// the engine refuses (its notes are too long); #120 a LIVE BUY of USD 100 at 32.2 by cash for
/// `CX-00000001` at 07:00:00.000Z; #123 a LIVE SELL of USD 20 at 32.7 by PromptPay with a slip.
const ENGINE_REFUSES: usize = 119;
const LIVE_BUY: usize = 120;
const LIVE_SELL: usize = 123;
const DEMO_BUY: usize = 125;
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

/// A booking row of Blueprint bookings table. `fields` overrides the defaults: a CHECKED_IN BUY of USD 100
/// in `MAIN`.
async fn book(replay: &Replay<'_>, id: &str, fields: Value) {
    let pool = &replay.desk.db.pool;
    let branch = fields
        .get("branch")
        .and_then(|v| v.as_str())
        .unwrap_or("MAIN");
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(branch)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|_| Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap());

    let slot_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{branch}-slot-2026-10-06").as_bytes(),
    );
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
         VALUES ($1, $2, $3, '2026-10-06', '2026-10-06 06:00:00+00', '2026-10-06 08:00:00+00', 10, 1, 'OPEN') \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(slot_id)
    .bind(org_id)
    .bind(branch)
    .execute(pool)
    .await
    .expect("seed slot for booking");

    let default_customer = payload_of(replay, LIVE_BUY)["customerId"]
        .as_str()
        .unwrap_or("CUST-00000001")
        .to_string();
    let customer_id = fields
        .get("customerId")
        .and_then(|v| v.as_str())
        .unwrap_or(&default_customer);
    let status = fields
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("CHECKED_IN");
    let side = fields.get("type").and_then(|v| v.as_str()).unwrap_or("BUY");
    let currency = fields.get("code").and_then(|v| v.as_str()).unwrap_or("USD");
    let amount_str = fields
        .get("amount")
        .and_then(|v| v.as_str())
        .unwrap_or("100");
    let amount: rust_decimal::Decimal = amount_str
        .parse()
        .unwrap_or(rust_decimal::Decimal::from(100));

    let tx_id: Option<Uuid> = fields
        .get("transactionId")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .or_else(|| (status == "COMPLETED").then(Uuid::new_v4));

    if let Some(t_id) = tx_id {
        sqlx::query(
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, status, submission_id, payload_hash, \
                customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
                recorded_at, recorded_by, completed_at, count_confirmed) VALUES ($1, $2, $3, $4, 'DEMO', 'COMPLETED', $5, 'payload-hash', $6, 'Test Customer', 'THA', \
                'teller-1', 'B261014-0001', '', '2026-10-14', now(), now(), 'teller-1', now(), true) ON CONFLICT (id) DO NOTHING",
        )
        .bind(t_id)
        .bind(org_id)
        .bind(format!("DOC-{t_id}"))
        .bind(branch)
        .bind(format!("sub-{t_id}"))
        .bind(customer_id)
        .execute(pool)
        .await
        .expect("seed tx for completed booking");
    }

    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    let hash = hasher.finalize().to_vec();

    let confirm_expires = (status == "PENDING_CUSTOMER_CONFIRMATION")
        .then(|| Utc::now() + chrono::Duration::minutes(15));
    let booking_no = format!(
        "B-{}",
        &id.replace('-', "")[..id.replace('-', "").len().min(8)]
    );

    sqlx::query(
        "INSERT INTO bookings (\
            id, organization_id, branch_id, booking_no, customer_id, slot_id, \
            shop_side, requested_currency, requested_quantity, source_channel, \
            status, transaction_id, qr_token_hash, qr_version, confirm_expires_at, created_by \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'APP', $10, $11, $12, 1, $13, 'owner') \
         ON CONFLICT (id) DO UPDATE SET \
            status = EXCLUDED.status, \
            transaction_id = EXCLUDED.transaction_id, \
            confirm_expires_at = EXCLUDED.confirm_expires_at",
    )
    .bind(id)
    .bind(org_id)
    .bind(branch)
    .bind(&booking_no)
    .bind(customer_id)
    .bind(slot_id)
    .bind(side)
    .bind(currency)
    .bind(amount)
    .bind(status)
    .bind(tx_id)
    .bind(&hash)
    .bind(confirm_expires)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("book {id}: {e}"));
}

/// The stored status of booking `id` and the transaction it became.
async fn booking_row(replay: &Replay<'_>, id: &str) -> Value {
    let (status, transaction_id): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status, transaction_id FROM bookings WHERE id = $1")
            .bind(id)
            .fetch_one(&replay.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("read booking {id}: {e}"));
    json!({ "status": status, "transaction_id": transaction_id })
}

fn untouched() -> Value {
    json!({ "status": "CHECKED_IN", "transaction_id": null })
}

fn completed_with(tx_id: Uuid) -> Value {
    json!({ "status": "COMPLETED", "transaction_id": tx_id })
}

/// The id v3 gave the LIVE customer with this passport.
async fn customer_with(replay: &Replay<'_>, passport: &str) -> String {
    // CI-4 (#651): one customer per company with an encrypted passport (#554 K1, K2).
    snapshot_common::live_customer_with_passport(&replay.desk.db.pool, passport)
        .await
        .unwrap_or_else(|| panic!("the customer with {passport}"))
}

async fn save(replay: &Replay<'_>, who: &str, route: &str, payload: &Value) -> Answer {
    // OR-4b (C2): a save attaches its saver's own files; the teller uploads the owner's again
    let payload = replay.uploads_as(who, payload).await;
    replay
        .app
        .send(Call::action(ORIGIN, route, "saveTransaction", payload).cookie(&replay.cookies[who]))
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

/// Re-expected for TX-6 by the test-writer, decision A3 (design quote-transaction.md §4.6.1 and §6.4, #674; the client-rate check at quote comes first, lane controller's ruling issuecomment-6116917556): a booking no longer sets the rate. A booked save
/// is quoted at the current published line like a walk-in, so a booking at 32.1 sent at 32.1 while
/// the counter buys at 32.2 is refused `RATE_MISMATCH`, and the booking stays as it was. (Before
/// TX-6 the booked rate stood and skipped the counter check.)
#[actix_web::test]
async fn a_booked_rate_off_the_counter_is_refused() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-rate", json!({ "rate": "32.1" })).await;
    let mut payload = claiming(&payload_of(&replay, LIVE_BUY), "BK-rate");
    payload["rate"] = json!(32.1);

    refused_and_kept(
        &replay,
        TELLER,
        RPC,
        &payload,
        RATE_MISMATCH,
        "BK-rate",
        "a booking at 32.1, the counter at 32.2 (decision A3)",
    )
    .await;
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
            TELLER,
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

/// One booking, two submissions: the first saves and completes it, the second is refused and keeps
/// nothing (design B10, row 6).
#[actix_web::test]
async fn second_submission_refused() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-once", json!({})).await;
    let first = claiming(&payload_of(&replay, LIVE_BUY), "BK-once");
    let document = saved_ref(&save(&replay, TELLER, RPC, &first).await, "the first save");
    let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
        .bind(&document)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("transaction id for saved document");
    assert_eq!(booking_row(&replay, "BK-once").await, completed_with(tx_id));

    let mut second = first.clone();
    second["submissionId"] = json!(FRESH_A);
    second["notes"] = json!("another exchange");
    refused_and_kept(
        &replay,
        TELLER,
        RPC,
        &second,
        USED_BY_ANOTHER,
        "BK-once",
        "a second submission with the same booking",
    )
    .await;
    replay.finish().await;
}

/// The same save sent again answers the stored exchange; the booking stays COMPLETED by it and
/// nothing is written again.
#[actix_web::test]
async fn repeat_passes() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    // Re-expected for TX-6 by the test-writer (decision A3 (design quote-transaction.md §4.6.1 and §6.4, #674; the client-rate check at quote comes first, lane controller's ruling issuecomment-6116917556)): the booking is at the counter rate 32.2,
    // because a booked save is quoted at the published line; the repeat is what this test holds.
    book(&replay, "BK-repeat", json!({ "rate": "32.2" })).await;
    let mut payload = claiming(&payload_of(&replay, LIVE_BUY), "BK-repeat");
    payload["rate"] = json!(32.2);
    let document = saved_ref(
        &save(&replay, TELLER, RPC, &payload).await,
        "the first save",
    );
    let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
        .bind(&document)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("transaction id for saved document");
    let before = kept(&replay).await;

    let again = save(&replay, TELLER, RPC, &payload).await;
    assert_eq!(
        saved_ref(&again, "the repeat"),
        document,
        "the stored answer"
    );
    assert_eq!(kept(&replay).await, before, "nothing written again");
    assert_eq!(
        booking_row(&replay, "BK-repeat").await,
        completed_with(tx_id)
    );
    replay.finish().await;
}

/// A booking is for that booking's customer only; another customer claiming it is refused
/// with `bookingNotThisCustomer` and nothing is written (design B10, R5).
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

    book(&replay, "BK-theirs", json!({ "customerId": other })).await;
    book(&replay, "BK-ours", json!({ "customerId": customer })).await;

    refused_and_kept(
        &replay,
        TELLER,
        RPC,
        &claiming(&payload, "BK-theirs"),
        NOT_THIS_CUSTOMER,
        "BK-theirs",
        "another customer's booking",
    )
    .await;

    let mine = claiming(&payload, "BK-ours");
    let document = saved_ref(
        &save(&replay, TELLER, RPC, &mine).await,
        "the customer's own booking",
    );
    let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
        .bind(&document)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("transaction id for saved document");
    assert_eq!(booking_row(&replay, "BK-ours").await, completed_with(tx_id));
    assert_eq!(booking_row(&replay, "BK-theirs").await, untouched());
    replay.finish().await;
}

/// A booking not in CHECKED_IN or IN_SERVICE status is refused with bookingNotUsable (design B10, R5).
#[actix_web::test]
async fn cancelled_or_expired_refused() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);

    for status in ["CANCELLED", "EXPIRED", "NO_SHOW", "CONFIRMED"] {
        let id = format!("BK-{status}");
        book(&replay, &id, json!({ "status": status })).await;
        refused_and_kept(
            &replay,
            TELLER,
            RPC,
            &claiming(&payload, &id),
            NOT_USABLE,
            &id,
            &format!("booking in {status} status"),
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
    sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ('B2', 'Branch Two', '00000000-0000-0000-0000-000000000001') ON CONFLICT (id) DO NOTHING")
        .execute(&replay.desk.db.pool)
        .await
        .expect("a second branch");
    book(&replay, "BK-elsewhere", json!({ "branch": "B2" })).await;
    book(&replay, "BK-here", json!({})).await;
    let payload = payload_of(&replay, LIVE_BUY);

    refused_and_kept(
        &replay,
        TELLER,
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
        TELLER,
        RPC,
        &nowhere,
        NOT_FOUND_HERE,
        "BK-here",
        "no booking with that id",
    )
    .await;
    replay.finish().await;
}

/// A save claiming a CHECKED_IN booking completes the booking with that save's transaction id,
/// and transactions.booking_id stays NULL per R1 (dell-claude-6 must-fix 3, R6).
#[actix_web::test]
async fn save_claiming_checked_in_booking_completes_with_transaction_id() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    book(&replay, "BK-save-path", json!({ "status": "CHECKED_IN" })).await;
    let payload = claiming(&payload_of(&replay, LIVE_BUY), "BK-save-path");

    let answer = save(&replay, TELLER, RPC, &payload).await;
    let document = saved_ref(&answer, "save claiming checked-in booking");

    let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
        .bind(&document)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("transaction id for saved document");

    // R1: transactions.booking_id stays NULL for Blueprint bookings
    let (stored_booking_id,): (Option<String>,) =
        sqlx::query_as("SELECT booking_id FROM transactions WHERE id = $1")
            .bind(tx_id)
            .fetch_one(&replay.desk.db.pool)
            .await
            .expect("read transactions.booking_id");
    assert!(
        stored_booking_id == Some("BK-save-path".to_owned()),
        "transactions.booking_id names the Blueprint booking (BK-11)"
    );

    assert_eq!(
        booking_row(&replay, "BK-save-path").await,
        completed_with(tx_id)
    );
    replay.finish().await;
}

/// A save claiming another customer's booking is refused with bookingNotThisCustomer,
/// and neither the transaction nor the booking is written (dell-claude-6 must-fix 3, R6).
#[actix_web::test]
async fn save_claiming_another_customers_booking_refuses_booking_not_this_customer() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);
    let other = customer_with(&replay, OTHER_PASSPORT).await;

    book(&replay, "BK-other-cust", json!({ "customerId": other })).await;

    refused_and_kept(
        &replay,
        TELLER,
        RPC,
        &claiming(&payload, "BK-other-cust"),
        NOT_THIS_CUSTOMER,
        "BK-other-cust",
        "save claiming another customer's booking",
    )
    .await;

    assert_eq!(
        booking_row(&replay, "BK-other-cust").await,
        untouched(),
        "booking remains untouched in CHECKED_IN status"
    );
    replay.finish().await;
}

/// A DEMO save cannot use a booking (design B10, ruling R9).
#[actix_web::test]
async fn demo_save_cannot_use_a_booking() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, DEMO_BUY - 1).await;
    book(&replay, "BK-demo", json!({ "status": "CHECKED_IN" })).await;
    let mut payload = claiming(&payload_of(&replay, DEMO_BUY), "BK-demo");
    assert_eq!(payload["mode"], json!("DEMO"));
    payload["rate"] = json!(32.2);
    refused_and_kept(
        &replay,
        TELLER,
        RPC,
        &payload,
        NOT_USABLE,
        "BK-demo",
        "a DEMO save naming a booking",
    )
    .await;
    assert_eq!(
        booking_row(&replay, "BK-demo").await,
        untouched(),
        "the booking stays CHECKED_IN"
    );
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
        &save(&replay, TELLER, RPC, &walk_in).await,
        "the walk-in save",
    );

    refused_and_kept(
        &replay,
        TELLER,
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
        &save(&replay, TELLER, RPC, &fresh).await,
        "a new submission with the booking",
    );
    let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
        .bind(&document)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("transaction id for saved document");
    assert_eq!(booking_row(&replay, "BK-kept").await, completed_with(tx_id));
    replay.finish().await;
}

/// Two desks save with one booking at the same moment — the owner on `/api/rpc` and a Front on
/// `/api/staff/rpc`, two sessions, so the two requests really run side by side: exactly one
/// exchange is saved, the other is refused with v2's sentence (not a server error), and the
/// booking is COMPLETED by the one saved. Done for [`RACES`] bookings in turn, so that a check made
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
            save(&replay, TELLER, RPC, &a),
            save(&replay, "FR1", STAFF_RPC, &b)
        );
        let (won, lost, _winner) = match (first.status, second.status) {
            (200, _) => (first, second, sub_a),
            (_, 200) => (second, first, sub_b),
            _ => panic!(
                "{id}: one of the two is saved: {:.300} / {:.300}",
                first.text, second.text
            ),
        };
        lost.assert_refused(400, USED_BY_ANOTHER, &format!("{id}: the other desk"));
        let document = saved_ref(&won, &format!("{id}: the desk that won"));
        let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
            .bind(&document)
            .fetch_one(&replay.desk.db.pool)
            .await
            .expect("transaction id for saved document");
        assert_eq!(
            booking_row(&replay, &id).await,
            completed_with(tx_id),
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

/// The staff route saves through the same action: a Front's save from a booking is saved, and uses
/// the booking.
///
/// Re-expected for TX-6 by the test-writer (decision A3 (design quote-transaction.md §4.6.1 and §6.4, #674; the client-rate check at quote comes first, lane controller's ruling issuecomment-6116917556)): the booking is at the counter's selling rate
/// 32.7, because a booked save is quoted at the published line (before TX-6 it was at 32.65, a rate
/// the counter no longer sold at). The save with no booking at 32.65 is still refused.
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
        json!({ "type": "SELL", "amount": "20", "rate": "32.7" }),
    )
    .await;
    let mut payload = claiming(&payload_of(&replay, LIVE_SELL), "BK-staff");
    payload["passport"] = json!({ "uploadKey": keys[0] });
    payload["slip"] = json!({ "uploadKey": keys[1] });
    payload["rate"] = json!(32.7);

    let mut no_claim = payload.clone();
    no_claim
        .as_object_mut()
        .expect("a payload")
        .remove("onlineBookingId");
    no_claim["rate"] = json!(32.65);
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
        "32.7",
        "saved at the booked rate, the counter's (decision A3)"
    );
    let tx_id: Uuid = sqlx::query_scalar("SELECT id FROM transactions WHERE document_ref = $1")
        .bind(&document)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("transaction id for saved document");
    assert_eq!(
        booking_row(&replay, "BK-staff").await,
        completed_with(tx_id)
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
        TELLER,
        RPC,
        &payload,
        &sentence,
        "BK-here",
        "a form the engine refuses, naming no booking of the branch",
    )
    .await;
    replay.finish().await;
}
