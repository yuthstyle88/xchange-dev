// Written before the implementation by the ticket author claude/dell-claude-6 (order from the xchange
// BE controller claude/a90ab16c). Do not edit, delete or weaken.

//! Up-front specification tests for the C03 booking wait estimate ticket (FE gap survey section B,
//! #564 issuecomment-6262359159; design `docs/design/blueprint/booking-slots.md:241`: `my_bookings`
//! shows the "ticket number and estimate (queue's `my_queue_ticket`)").
//!
//! The estimate is the queue's own (`StatusService::estimate_for_in`): `ahead ÷ active counters ×`
//! the 0.5 / 0.25 / 0.75 percentiles of the sample (earlier days' last 30 COMPLETED service times
//! here). Every sample below is 300 s, so all three are `ahead × 300 ÷ counters`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use bk6_common::*;

/// Branch B01 in Bangkok time, with exactly `counters` ACTIVE counters and three COMPLETED walk-in tickets of
/// yesterday, each served for 300 s.
async fn queue_with_history(pool: &PgPool, counters: usize) {
    sqlx::query("UPDATE branches SET timezone = 'Asia/Bangkok' WHERE id = $1")
        .bind(B01)
        .execute(pool)
        .await
        .expect("branch timezone");
    let yesterday = bangkok_today() - Duration::days(1);
    for seq in 1..=3 {
        let served = Utc::now() - Duration::days(1) - Duration::minutes(60 - seq as i64);
        ticket(
            pool,
            yesterday,
            "WALK_IN",
            seq,
            "COMPLETED",
            None,
            Some(served),
            Some(served + Duration::seconds(300)),
        )
        .await;
    }
    // exactly `counters` ACTIVE counters: the harness may already have opened one
    sqlx::query("DELETE FROM queue_counters WHERE branch_id = $1")
        .bind(B01)
        .execute(pool)
        .await
        .expect("clear the counters");
    for teller in [TELLER_ID, MANAGER_ID].iter().take(counters) {
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1::uuid, $2, $3::uuid, 'ACTIVE')",
        )
        .bind(ORG)
        .bind(B01)
        .bind(teller)
        .execute(pool)
        .await
        .expect("an active counter");
    }
}

#[allow(clippy::too_many_arguments)]
async fn ticket(
    pool: &PgPool,
    date: chrono::NaiveDate,
    service_type: &str,
    sequence: i32,
    status: &str,
    booking_id: Option<&str>,
    served_at: Option<chrono::DateTime<Utc>>,
    completed_at: Option<chrono::DateTime<Utc>>,
) {
    let prefix = if service_type == "BOOKING" { "B" } else { "W" };
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
         display_number, status, booking_id, issued_at, served_at, completed_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9, now(), $10, $11, $12)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG)
    .bind(B01)
    .bind(date)
    .bind(service_type)
    .bind(sequence)
    .bind(format!("{prefix}{sequence:03}"))
    .bind(status)
    .bind(booking_id)
    .bind(served_at)
    .bind(completed_at)
    .bind(unique("qt"))
    .execute(pool)
    .await
    .expect("a queue ticket");
}

/// A CONFIRMED booking of `customer` in a fresh slot of today; answers its id.
async fn booking(pool: &PgPool, id: &str, no: &str, customer: &str) -> String {
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        5,
        1,
        "OPEN",
    )
    .await;
    let qr = Sha256::digest(id.as_bytes());
    seed_booking(
        pool,
        id,
        no,
        customer,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr,
    )
    .await;
    id.to_owned()
}

/// Alice's booking `id` as `my_bookings` answers it.
async fn alices(db: &TestDb, id: &str) -> Value {
    let uri = format!("/api/v3/views/my_bookings?branch={B01}&bookingId={id}");
    let res = call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "my_bookings: {}", res.text);
    let body: Value = serde_json::from_str(&res.text).expect("JSON");
    body["data"]["bookings"]
        .as_array()
        .and_then(|b| b.iter().find(|r| r["id"] == id).cloned())
        .unwrap_or_else(|| panic!("booking {id} in my_bookings: {}", res.text))
}

/// 1. A WAITING booking ticket with two booking tickets ahead and one counter: ahead 2, every figure 600 s.
#[actix_web::test]
async fn c03_a_waiting_ticket_shows_ahead_and_the_queue_estimate() {
    let db = TestDb::start().await;
    queue_with_history(&db.pool, 1).await;
    let today = bangkok_today();
    let b1 = booking(
        &db.pool,
        "BK-c03-ahead-1",
        "B-C03-0001",
        CUSTOMER_BOB_CUST_ID,
    )
    .await;
    let b2 = booking(
        &db.pool,
        "BK-c03-ahead-2",
        "B-C03-0002",
        CUSTOMER_BOB_CUST_ID,
    )
    .await;
    let mine = booking(
        &db.pool,
        "BK-c03-mine-1",
        "B-C03-0003",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        1,
        "WAITING",
        Some(&b1),
        None,
        None,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        2,
        "WAITING",
        Some(&b2),
        None,
        None,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        3,
        "WAITING",
        Some(&mine),
        None,
        None,
    )
    .await;

    let row = alices(&db, &mine).await;
    assert_eq!(row["ticketNumber"], "B003", "{row}");
    assert_eq!(row["ticketsAhead"], 2, "{row}");
    assert_eq!(
        row["estimateSeconds"], 600,
        "2 ahead × 300 s ÷ 1 counter: {row}"
    );
    assert_eq!(row["estimateLowSeconds"], 600, "{row}");
    assert_eq!(row["estimateHighSeconds"], 600, "{row}");
    assert!(
        row["estimateAsOf"].is_string(),
        "the time the estimate was taken: {row}"
    );
    db.finish().await;
}

/// 2. Two active counters halve the wait.
#[actix_web::test]
async fn c03_two_counters_halve_the_estimate() {
    let db = TestDb::start().await;
    queue_with_history(&db.pool, 2).await;
    let today = bangkok_today();
    let b1 = booking(&db.pool, "BK-c03-2c-1", "B-C03-0011", CUSTOMER_BOB_CUST_ID).await;
    let b2 = booking(&db.pool, "BK-c03-2c-2", "B-C03-0012", CUSTOMER_BOB_CUST_ID).await;
    let mine = booking(
        &db.pool,
        "BK-c03-2c-mine",
        "B-C03-0013",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        1,
        "WAITING",
        Some(&b1),
        None,
        None,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        2,
        "WAITING",
        Some(&b2),
        None,
        None,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        3,
        "WAITING",
        Some(&mine),
        None,
        None,
    )
    .await;

    let row = alices(&db, &mine).await;
    assert_eq!(row["ticketsAhead"], 2, "{row}");
    assert_eq!(
        row["estimateSeconds"], 300,
        "2 ahead × 300 s ÷ 2 counters: {row}"
    );
    db.finish().await;
}

/// 3. No active counter: the queue knows the place but cannot estimate, so the figures are null.
#[actix_web::test]
async fn c03_no_active_counter_gives_ahead_but_no_estimate() {
    let db = TestDb::start().await;
    queue_with_history(&db.pool, 0).await;
    let today = bangkok_today();
    let b1 = booking(&db.pool, "BK-c03-nc-1", "B-C03-0021", CUSTOMER_BOB_CUST_ID).await;
    let mine = booking(
        &db.pool,
        "BK-c03-nc-mine",
        "B-C03-0022",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        1,
        "WAITING",
        Some(&b1),
        None,
        None,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        2,
        "WAITING",
        Some(&mine),
        None,
        None,
    )
    .await;

    let row = alices(&db, &mine).await;
    assert_eq!(row["ticketsAhead"], 1, "{row}");
    assert!(row["estimateSeconds"].is_null(), "{row}");
    assert!(row["estimateLowSeconds"].is_null(), "{row}");
    assert!(row["estimateHighSeconds"].is_null(), "{row}");
    db.finish().await;
}

/// 4. A walk-in ticket is never ahead of a booking ticket (call order Q6).
#[actix_web::test]
async fn c03_walk_ins_are_not_ahead_of_a_booking() {
    let db = TestDb::start().await;
    queue_with_history(&db.pool, 1).await;
    let today = bangkok_today();
    let mine = booking(
        &db.pool,
        "BK-c03-wi-mine",
        "B-C03-0031",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    ticket(&db.pool, today, "WALK_IN", 1, "WAITING", None, None, None).await;
    ticket(&db.pool, today, "WALK_IN", 2, "WAITING", None, None, None).await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        5,
        "WAITING",
        Some(&mine),
        None,
        None,
    )
    .await;

    let row = alices(&db, &mine).await;
    assert_eq!(row["ticketsAhead"], 0, "{row}");
    assert_eq!(row["estimateSeconds"], 0, "nobody ahead: no wait: {row}");
    db.finish().await;
}

/// 5. A ticket already CALLED: no place in the queue and no estimate, as `my_queue_ticket` answers.
#[actix_web::test]
async fn c03_a_called_ticket_has_no_wait() {
    let db = TestDb::start().await;
    queue_with_history(&db.pool, 1).await;
    let today = bangkok_today();
    let mine = booking(
        &db.pool,
        "BK-c03-called",
        "B-C03-0041",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;
    ticket(
        &db.pool,
        today,
        "BOOKING",
        1,
        "CALLED",
        Some(&mine),
        None,
        None,
    )
    .await;

    let row = alices(&db, &mine).await;
    assert_eq!(row["ticketNumber"], "B001", "{row}");
    assert_eq!(row["ticketsAhead"], 0, "{row}");
    assert!(row["estimateSeconds"].is_null(), "{row}");
    assert!(row["estimateAsOf"].is_null(), "{row}");
    db.finish().await;
}

/// 6. A booking with no ticket yet: every queue figure is null.
#[actix_web::test]
async fn c03_no_ticket_means_no_queue_figures() {
    let db = TestDb::start().await;
    queue_with_history(&db.pool, 1).await;
    let mine = booking(
        &db.pool,
        "BK-c03-none",
        "B-C03-0051",
        CUSTOMER_ALICE_CUST_ID,
    )
    .await;

    let row = alices(&db, &mine).await;
    assert!(row["ticketNumber"].is_null(), "{row}");
    assert!(row["ticketsAhead"].is_null(), "{row}");
    assert!(row["estimateSeconds"].is_null(), "{row}");
    assert!(row["estimateAsOf"].is_null(), "{row}");
    db.finish().await;
}
