//! XC-BE-GAPS-A row 7 (BK-14, design `docs/design/blueprint/booking-slots.md` §5.2 `:437-449`, `:573`): the QR
//! re-issue (`bookingQrReissue`, `POST /api/bookings/{id}/qr`) reads the overdue rule through `effective_status`,
//! so a booking the sweeper has not reached yet gets no new QR.
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 7, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Pinned here:
//! - §5.2 `:445-447`: the QR is withheld when the effective status is `CANCELLED`, `COMPLETED`, `NO_SHOW` or
//!   `EXPIRED`; `qr.rs` calls `effective_status` (`xc-booking/src/domain/booking/effective_status.rs`) so the rule
//!   has one home (`:448-451`, BK-14 `:573`).
//! - The refusal is today's for a stored final status: 409 `staleVersion` (`qr.rs`), and nothing is written: the
//!   stored hash, `qr_version` and status stay as they were. Writing the final status stays the sweeper's (B12).
//! - Each refusal sits beside a live twin that still gets its QR (200 + `qrToken`, `qr_version` + 1), so the rule
//!   cannot be met by refusing more than §5.2 says; and beside the stored final status that §5.2 maps it to, which
//!   is refused as today.
//!
//! Not pinned here: the boundary instants (`now == grace_until`, `now == confirm_expires_at`): those are
//! `crates/xc-booking/tests/effective_status_test.rs`'s. The organisation's grace is 15 minutes; every overdue case
//! is far past its boundary and every live one well inside it. A `COMPLETED` row needs a transaction
//! (`bookings` CHECK), so the stored-final guards use `NO_SHOW`, `EXPIRED` and `CANCELLED`.
//!
//! The caller is the booking's own customer (`CUSTOMER@customer-alice`), naming `branch` = `XC-B01`.
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

/// The organisation's booking policy, with a 15-minute no-show grace.
async fn grace_of_15_minutes(db: &TestDb) {
    sqlx::query(
        "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, \
             confirm_expiry_minutes) VALUES ($1::uuid, 60, 15, 60) \
         ON CONFLICT (organization_id) DO UPDATE SET no_show_grace_minutes = 15",
    )
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("the booking policy");
}

/// A slot of `B01` on `date`, from `start` for an hour.
async fn slot(db: &TestDb, date: NaiveDate, start: DateTime<Utc>) -> Uuid {
    let id = Uuid::new_v4();
    seed_slot(
        &db.pool,
        id,
        B01,
        date,
        start,
        start + Duration::hours(1),
        5,
        1,
        "OPEN",
    )
    .await;
    id
}

/// A booking of Alice's in `slot_id` with stored `status`; answers its id (`BK-<uuid>`, which the re-issue parses).
async fn booking(db: &TestDb, no: &str, slot_id: Uuid, status: &str) -> String {
    let id = format!("BK-{}", Uuid::new_v4());
    let qr_hash = Sha256::digest(id.as_bytes());
    seed_booking(
        &db.pool,
        &id,
        no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        status,
        &qr_hash,
    )
    .await;
    id
}

/// What a re-issue may change: the stored hash, `qr_version`, and the stored status.
async fn stored(db: &TestDb, id: &str) -> (Vec<u8>, i32, String) {
    sqlx::query_as("SELECT qr_token_hash, qr_version, status FROM bookings WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the stored booking")
}

async fn reissue(db: &TestDb, id: &str) -> Answer {
    let uri = format!("/api/bookings/{id}/qr");
    let key = unique("k-gapsa7");
    call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(json!({ "branch": B01 })),
        Some(&key),
    )
    .await
}

/// 409 `staleVersion`, and nothing written.
async fn refused_and_untouched(db: &TestDb, id: &str, what: &str) {
    let before = stored(db, id).await;
    let res = reissue(db, id).await;
    assert_eq!(
        (res.status, res.body["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("staleVersion")),
        "{what}: expected 409 staleVersion; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(
        stored(db, id).await,
        before,
        "{what}: a refused re-issue writes nothing (hash, qr_version, status)"
    );
}

/// 200 with a new QR: `qrToken` is a string and `qr_version` went up by one.
async fn reissued(db: &TestDb, id: &str, what: &str) {
    let (_, version_before, status_before) = stored(db, id).await;
    let res = reissue(db, id).await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "{what}: expected 200 and a new QR; got {} {}",
        res.status,
        res.text
    );
    assert!(
        res.body["qrToken"].is_string(),
        "{what}: the answer carries the new QR: {}",
        res.text
    );
    let (_, version_after, status_after) = stored(db, id).await;
    assert_eq!(version_after, version_before + 1, "{what}: qr_version + 1");
    assert_eq!(status_after, status_before, "{what}: the status is kept");
}

#[actix_web::test]
async fn gapsa7_a_confirmed_booking_past_start_plus_grace_gets_no_new_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();

    // Two hours after its start: effective NO_SHOW; the stored row is still CONFIRMED.
    let overdue = slot(&db, bangkok_today(), now - Duration::hours(2)).await;
    let overdue_id = booking(&db, "B261027-7001", overdue, "CONFIRMED").await;
    refused_and_untouched(&db, &overdue_id, "CONFIRMED two hours past its start").await;
    assert_eq!(
        stored(&db, &overdue_id).await.2,
        "CONFIRMED",
        "the refusal does not write NO_SHOW: the sweeper writes"
    );

    // Started five minutes ago, inside the 15-minute grace: still CONFIRMED, as today.
    let started = slot(&db, bangkok_today(), now - Duration::minutes(5)).await;
    let started_id = booking(&db, "B261027-7002", started, "CONFIRMED").await;
    reissued(&db, &started_id, "CONFIRMED five minutes after its start").await;

    // The stored NO_SHOW that §5.2 maps the overdue row to is refused, as today.
    let stored_no_show = slot(&db, bangkok_today(), now - Duration::hours(3)).await;
    let stored_no_show_id = booking(&db, "B261027-7003", stored_no_show, "NO_SHOW").await;
    refused_and_untouched(&db, &stored_no_show_id, "a stored NO_SHOW").await;

    db.finish().await;
}

#[actix_web::test]
async fn gapsa7_a_pending_booking_past_its_confirm_expiry_gets_no_new_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();
    let later = slot(&db, bangkok_today(), now + Duration::hours(3)).await;

    // Half an hour past its confirm expiry: effective EXPIRED; stored PENDING_CUSTOMER_CONFIRMATION.
    let expired_id = booking(&db, "B261027-7011", later, "PENDING_CUSTOMER_CONFIRMATION").await;
    sqlx::query("UPDATE bookings SET confirm_expires_at = $2 WHERE id = $1")
        .bind(&expired_id)
        .bind(now - Duration::minutes(30))
        .execute(&db.pool)
        .await
        .expect("an expired confirmation window");
    refused_and_untouched(
        &db,
        &expired_id,
        "PENDING half an hour past its confirm expiry",
    )
    .await;
    assert_eq!(
        stored(&db, &expired_id).await.2,
        "PENDING_CUSTOMER_CONFIRMATION",
        "the refusal does not write EXPIRED: the sweeper writes"
    );

    // Inside its window (`seed_booking` sets now + 1 h): still PENDING, as today.
    let live_id = booking(&db, "B261027-7012", later, "PENDING_CUSTOMER_CONFIRMATION").await;
    reissued(&db, &live_id, "PENDING inside its confirm window").await;

    // The stored EXPIRED is refused, as today.
    let stored_expired_id = booking(&db, "B261027-7013", later, "EXPIRED").await;
    refused_and_untouched(&db, &stored_expired_id, "a stored EXPIRED").await;

    db.finish().await;
}

/// A WAITING `BOOKING` ticket of `booking_id` on `date`: the booking's "active ticket" (§5.2 `:434`).
async fn active_ticket(db: &TestDb, booking_id: &str, date: NaiveDate) {
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
         display_number, status, booking_id, issued_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, $4, 'BOOKING', 1, 'B001', 'WAITING', $5, now(), $6)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG)
    .bind(B01)
    .bind(date)
    .bind(booking_id)
    .bind(unique("qt-gapsa7"))
    .execute(&db.pool)
    .await
    .expect("an active queue ticket");
}

#[actix_web::test]
async fn gapsa7_a_checked_in_booking_of_an_earlier_day_with_no_active_ticket_gets_no_new_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();
    let yesterday = bangkok_today() - Duration::days(1);

    // Yesterday's slot, no active ticket: effective CANCELLED (`dayEnded`); stored CHECKED_IN.
    let earlier = slot(&db, yesterday, now - Duration::hours(26)).await;
    let ended_id = booking(&db, "B261027-7021", earlier, "CHECKED_IN").await;
    refused_and_untouched(
        &db,
        &ended_id,
        "CHECKED_IN on yesterday's slot, no active ticket",
    )
    .await;
    assert_eq!(
        stored(&db, &ended_id).await.2,
        "CHECKED_IN",
        "the refusal does not write CANCELLED: the sweeper writes"
    );

    // Today's slot (started an hour ago, past the grace: CHECKED_IN has no grace rule): still CHECKED_IN.
    let today = slot(&db, bangkok_today(), now - Duration::hours(1)).await;
    let today_id = booking(&db, "B261027-7022", today, "CHECKED_IN").await;
    reissued(&db, &today_id, "CHECKED_IN today").await;

    // Yesterday's slot with a ticket still WAITING: the day has not ended for it, still CHECKED_IN.
    let kept_id = booking(&db, "B261027-7023", earlier, "CHECKED_IN").await;
    active_ticket(&db, &kept_id, yesterday).await;
    reissued(
        &db,
        &kept_id,
        "CHECKED_IN on yesterday's slot with an active ticket",
    )
    .await;

    // The stored CANCELLED is refused, as today.
    let stored_cancelled_id = booking(&db, "B261027-7024", earlier, "CANCELLED").await;
    refused_and_untouched(&db, &stored_cancelled_id, "a stored CANCELLED").await;

    db.finish().await;
}
