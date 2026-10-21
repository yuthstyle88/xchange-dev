//! Up-front tests for FD-2 (#973; design `docs/design/blueprint/broker-seam.md` §6.2): every LIVE
//! exchange the desk saves writes one `broker_exchange_feed` row in the save's transaction.
//! Written by the ticket author before the implementation. Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! The saves go through the real route (`saveTransaction` on `/api/rpc`), so these tests also prove
//! the step is wired into the compat save, after the booking step.
//!
//! The day of `rpc/engine_actions.json` is replayed up to the save under test, as in
//! `booking_save_step_test.rs`. Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing
//! here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod shift_holder_common;
mod snapshot_common;

use compat_common::Fixture;
use rpc_common::{Answer, Call, Desk, RPC, Replay};
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const FILE: &str = "rpc/engine_actions.json";
/// Who saves at the desk: the day's FRONT, FR1. v2's owner saved; the Blueprint's
/// `ExchangeTransact` is TELLER's only (OR-4b #767, design §4.2 C2), and B1 makes FRONT a TELLER.
const TELLER: &str = "FR1";

/// `engine_actions.json`: #120 a LIVE BUY of USD 100 at 32.2 by cash (principal 3220.00);
/// #125 a DEMO BUY of USD 100 at 1.
const LIVE_BUY: usize = 120;
const DEMO_BUY: usize = 125;
/// A save at a rate other than the published one (decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): DEMO too).
const RATE_MISMATCH: &str = "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ";
/// BK-10 #1412: the member's phone is the customer's verified ACTIVE `PHONE` identity, which the
/// feed carries in E.164 (ruling 3, issuecomment-6140233542); v2's `0899000111` is `+66899000111`.
const MEMBER_PHONE: &str = "+66899000111";

async fn day_up_to(fixture: &Fixture, last: usize) -> Replay<'_> {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
    for n in 1..=last {
        let step = fixture.step(n);
        if step.action.as_deref() == Some("saveTransaction") && step.sent_as == "owner" {
            let payload = payload_of(&replay, n);
            save(&replay, &payload).await;
        } else {
            replay.step(step).await;
        }
    }
    replay
}

fn payload_of(replay: &Replay<'_>, n: usize) -> Value {
    let step = replay.fixture.step(n);
    replay.desk.clock.set(time(&step.at));
    replay
        .ids
        .to_actual(step.payload.as_ref().expect("a recorded payload"))
}

/// BK-10 #1412: `customer` becomes a member — a verified ACTIVE `PHONE` identity with phone
/// [`MEMBER_PHONE`], sealed as every `PHONE` identity on `main` is (`ContactReserve`): the E.164
/// form, AAD `"{customer_id}PHONE"`, `lookup_hash = LookupHasher::hash(IdentityType::Phone, e164)`,
/// with the PII key the harness installed.
async fn verified_member(replay: &Replay<'_>, customer: &str) {
    use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher};
    let keys = xc_customer::get_pii_keys().expect("the harness installed a PII key");
    let sealed = PiiCipher::new(keys.clone()).encrypt(
        format!("{customer}PHONE").as_bytes(),
        MEMBER_PHONE.as_bytes(),
    );
    let lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, MEMBER_PHONE);
    sqlx::query(
        "INSERT INTO customer_identities (id, customer_id, organization_id, type, encrypted_value, \
         key_id, lookup_hash, verified_at, status, created_at) \
         SELECT gen_random_uuid(), c.id, c.organization_id, 'PHONE', $2, $3, $4, \
         '2026-10-06T06:00:00.000Z'::timestamptz, 'ACTIVE', '2026-10-06T06:00:00.000Z'::timestamptz \
         FROM customers c WHERE c.id = $1",
    )
    .bind(customer)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&lookup_hash[..])
    .execute(&replay.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("the verified phone of {customer}: {e}"));
}

/// BK-10 #1412: a Blueprint booking `id` of `customer` at `MAIN`, CHECKED_IN, which the next
/// exchange of `customer` completes. Nothing on the save path sets `bookings.transaction_id` yet:
/// BK-7 #1405's `booking_save_step` will write it on completion (lane controller,
/// issuecomment-6146073973). The feed row is written inside the save's transaction, so the link
/// cannot be seeded after the save; a trigger stands in for BK-7 instead, linking the booking to
/// the exchange (and completing it) the moment the exchange's row is inserted, in that same
/// transaction, before the feed step reads it.
async fn blueprint_booking_completed_by_the_next_exchange(
    replay: &Replay<'_>,
    id: &str,
    customer: &str,
) {
    let pool = &replay.desk.db.pool;
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
         capacity, reserved, status) \
         SELECT '00000000-0000-4000-8000-00000000f00d'::uuid, b.organization_id, b.id, \
         DATE '2026-10-06', '2026-10-06T06:00:00.000Z'::timestamptz, \
         '2026-10-06T06:30:00.000Z'::timestamptz, 4, 1, 'OPEN' FROM branches b WHERE b.id = 'MAIN'",
    )
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("the slot of {id}: {e}"));
    sqlx::query(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
         shop_side, requested_currency, requested_quantity, source_channel, status, qr_token_hash, \
         checked_in_at, created_by) \
         SELECT $1, b.organization_id, b.id, 'B261006-0001', $2, \
         '00000000-0000-4000-8000-00000000f00d'::uuid, 'BUY', 'USD', 100, 'DESK', 'CHECKED_IN', \
         convert_to($1, 'UTF8'), '2026-10-06T06:00:00.000Z'::timestamptz, 'owner' \
         FROM branches b WHERE b.id = 'MAIN'",
    )
    .bind(id)
    .bind(customer)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("book {id}: {e}"));
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION bk7_stand_in_complete_booking() RETURNS trigger LANGUAGE plpgsql AS $f$ \
         BEGIN \
             UPDATE bookings SET status = 'COMPLETED', transaction_id = NEW.id, updated_at = now() \
             WHERE id = '{id}' AND customer_id = NEW.customer_id AND transaction_id IS NULL; \
             RETURN NEW; \
         END $f$; \
         CREATE TRIGGER bk7_stand_in_complete_booking AFTER INSERT ON transactions \
             FOR EACH ROW EXECUTE FUNCTION bk7_stand_in_complete_booking();"
    )))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("the BK-7 stand-in trigger: {e}"));
}

async fn save(replay: &Replay<'_>, payload: &Value) -> Answer {
    // OR-4b (C2): a save attaches its saver's own files; the teller uploads the owner's again
    let payload = replay.uploads_as(TELLER, payload).await;
    // CA-10 (#626 ruling A): a compat save meets the teller gate, which saves only on an OPEN day
    let branch = payload["branchId"].as_str().unwrap_or("MAIN");
    shift_holder_common::open_day(&replay.desk.db.pool, branch).await;
    replay
        .app
        .send(Call::action(ORIGIN, RPC, "saveTransaction", payload).cookie(&replay.cookies[TELLER]))
        .await
}

fn saved_ref(answer: &Answer, label: &str) -> String {
    assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
    answer.json()["documentRef"]
        .as_str()
        .unwrap_or_else(|| panic!("{label}: a document number: {:.300}", answer.text))
        .to_string()
}

#[derive(sqlx::FromRow)]
struct FeedRow {
    transaction_id: String,
    organization_id: String,
    branch_id: String,
    customer_sub: Option<String>,
    phone: Option<String>,
    status: String,
    value_satang: i64,
    reversed_satang: i64,
    at_ok: bool,
    org_ok: bool,
    value_ok: bool,
}

/// The feed row of exchange `document`, compared field by field with the exchange itself.
async fn feed_of(replay: &Replay<'_>, document: &str) -> Option<Value> {
    let row: Option<FeedRow> = sqlx::query_as(
        "SELECT f.transaction_id::text AS transaction_id, \
                f.organization_id::text AS organization_id, f.branch_id, f.customer_sub, \
                f.phone, f.status, f.value_satang, f.reversed_satang, \
                f.completed_at = t.recorded_at AS at_ok, \
                f.organization_id = b.organization_id AS org_ok, \
                f.value_satang = (SELECT (sum(l.thb_amount) * 100)::bigint FROM transaction_lines l \
                                  WHERE l.transaction_id = t.id) AS value_ok \
         FROM transactions t \
         JOIN broker_exchange_feed f ON f.transaction_id = t.id \
         JOIN branches b ON b.id = t.branch_id \
         WHERE t.document_ref = $1",
    )
    .bind(document)
    .fetch_optional(&replay.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("read the feed of {document}: {e}"));
    row.map(|r| {
        json!({
            "transaction_id": r.transaction_id, "organization_id": r.organization_id,
            "branch_id": r.branch_id, "customer_sub": r.customer_sub, "phone": r.phone,
            "status": r.status, "value_satang": r.value_satang,
            "reversed_satang": r.reversed_satang,
            "completed_at_is_recorded_at": r.at_ok,
            "organization_is_the_branchs": r.org_ok,
            "value_is_principal_times_100": r.value_ok,
        })
    })
}

async fn feed_rows(replay: &Replay<'_>) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM broker_exchange_feed")
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("count the feed")
}

async fn exchanges_of(replay: &Replay<'_>, submission: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM transactions WHERE submission_id = $1")
        .bind(submission)
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("count the exchanges")
}

fn assert_completed_row(row: &Value, phone: Option<&str>, label: &str) {
    assert_eq!(row["status"], json!("COMPLETED"), "{label}: {row}");
    assert_eq!(row["branch_id"], json!("MAIN"), "{label}: {row}");
    assert_eq!(
        row["customer_sub"],
        Value::Null,
        "{label}: no customer_sub before BK-11: {row}"
    );
    assert_eq!(row["phone"], json!(phone), "{label}: {row}");
    assert_eq!(
        row["value_satang"],
        json!(322_000),
        "{label}: 3220.00 THB: {row}"
    );
    assert_eq!(row["reversed_satang"], json!(0), "{label}: {row}");
    for check in [
        "completed_at_is_recorded_at",
        "organization_is_the_branchs",
        "value_is_principal_times_100",
    ] {
        assert_eq!(row[check], json!(true), "{label}: {check}: {row}");
    }
}

/// A LIVE walk-in save writes one COMPLETED row: its value is the principal in satang, its time
/// the exchange's `recorded_at`, its organisation the branch's, and it carries no customer keys.
#[actix_web::test]
async fn a_live_walk_in_save_writes_one_feed_row() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let before = feed_rows(&replay).await;
    let document = saved_ref(&save(&replay, &payload_of(&replay, LIVE_BUY)).await, "#120");
    let row = feed_of(&replay, &document)
        .await
        .expect("a feed row for #120");
    assert_completed_row(&row, None, "walk-in");
    assert_eq!(feed_rows(&replay).await, before + 1, "exactly one row");
    replay.finish().await;
}

/// The replayed day already holds LIVE exchanges saved through the route: each has its row, and
/// no DEMO exchange has one.
#[actix_web::test]
async fn every_live_exchange_of_the_day_has_a_row_and_no_demo_one_does() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, DEMO_BUY).await;
    let (live_without, demo_with): (i64, i64) = sqlx::query_as(
        "SELECT \
           count(*) FILTER (WHERE t.mode = 'LIVE' AND f.transaction_id IS NULL), \
           count(*) FILTER (WHERE t.mode <> 'LIVE' AND f.transaction_id IS NOT NULL) \
         FROM transactions t LEFT JOIN broker_exchange_feed f ON f.transaction_id = t.id",
    )
    .fetch_one(&replay.desk.db.pool)
    .await
    .expect("compare the exchanges with the feed");
    assert_eq!(live_without, 0, "every LIVE exchange has its feed row");
    assert_eq!(demo_with, 0, "no DEMO exchange has a feed row");
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM transactions WHERE mode = 'LIVE'")
        .fetch_one(&replay.desk.db.pool)
        .await
        .expect("count LIVE");
    assert!(live > 0, "the day holds LIVE exchanges");
    replay.finish().await;
}

/// A DEMO save writes nothing to the feed.
#[actix_web::test]
async fn a_demo_save_writes_no_feed_row() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, DEMO_BUY - 1).await;
    let before = feed_rows(&replay).await;
    // Re-expected for TX-6 by the test-writer, decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): a DEMO save is quoted like a LIVE one,
    // so #125's rate 1 is refused; the save is sent at the published 32.2 and still writes no feed row.
    let mut payload = payload_of(&replay, DEMO_BUY);
    save(&replay, &payload).await.assert_refused(
        400,
        RATE_MISMATCH,
        "#125 at rate 1 (decision A2)",
    );
    payload["rate"] = json!(32.2);
    let document = saved_ref(&save(&replay, &payload).await, "#125");
    assert_eq!(
        feed_of(&replay, &document).await,
        None,
        "no row for a DEMO exchange"
    );
    assert_eq!(feed_rows(&replay).await, before, "the feed is unchanged");
    replay.finish().await;
}

/// A save from a booking of a verified member carries the member's phone: the step reads the
/// booking the exchange completed. Re-pinned for BK-10 #1412 by claude/29df2ec5 (lane controller,
/// issuecomment-6146073973): the member is the booking's customer with a verified ACTIVE `PHONE`
/// identity, the booking a Blueprint one linked by `bookings.transaction_id`; the phone is E.164.
#[actix_web::test]
async fn a_booking_of_a_verified_member_carries_the_members_phone() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);
    let customer = payload["customerId"]
        .as_str()
        .expect("the customer")
        .to_string();
    verified_member(&replay, &customer).await;
    blueprint_booking_completed_by_the_next_exchange(&replay, "BK-member", &customer).await;
    let document = saved_ref(&save(&replay, &payload).await, "the member's booking");
    let row = feed_of(&replay, &document).await.expect("a feed row");
    assert_completed_row(&row, Some(MEMBER_PHONE), "verified member");
    replay.finish().await;
}

/// The same submission answered again from the stored exchange changes nothing in the feed.
#[actix_web::test]
async fn the_same_submission_answered_again_changes_nothing() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let payload = payload_of(&replay, LIVE_BUY);
    let document = saved_ref(&save(&replay, &payload).await, "first");
    let first = feed_of(&replay, &document).await.expect("a feed row");
    let updated_at = |pool| async move {
        sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT updated_at FROM broker_exchange_feed",
        )
        .fetch_all(pool)
        .await
        .expect("read updated_at")
    };
    let stamps = updated_at(&replay.desk.db.pool).await;
    let count = feed_rows(&replay).await;

    let again = saved_ref(&save(&replay, &payload).await, "again");
    assert_eq!(again, document, "the stored exchange answers");
    assert_eq!(
        feed_of(&replay, &document).await,
        Some(first),
        "the row is unchanged"
    );
    assert_eq!(feed_rows(&replay).await, count, "no second row");
    assert_eq!(
        updated_at(&replay.desk.db.pool).await,
        stamps,
        "no row was rewritten"
    );
    replay.finish().await;
}

/// A feed write that fails undoes the save: no exchange is kept for the submission.
#[actix_web::test]
async fn a_failed_feed_write_undoes_the_save() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    // Every new feed row is now refused by the database.
    sqlx::query(
        "ALTER TABLE broker_exchange_feed ADD CONSTRAINT fd2_test_refuses_every_row \
         CHECK (value_satang < 0) NOT VALID",
    )
    .execute(&replay.desk.db.pool)
    .await
    .expect("add the refusing constraint");
    let payload = payload_of(&replay, LIVE_BUY);
    let submission = payload["submissionId"]
        .as_str()
        .expect("a submission")
        .to_string();

    let answer = save(&replay, &payload).await;
    assert_ne!(answer.status, 200, "the save fails: {:.300}", answer.text);
    assert_eq!(
        exchanges_of(&replay, &submission).await,
        0,
        "the exchange was not kept"
    );
    replay.finish().await;
}
