//! XC-BE-GAPS-A row 9 (owner ruling R2, 2026-10-27: "kioskBookingCheckIn becomes staff-only"): the booking check-in
//! at `POST /api/kiosk/bookings/check-in` issues a BOOKING queue number only to staff who operate the queue.
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 9, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Why (design `docs/design/blueprint/kiosk-shared-phone.md`): R-1 (`:23`) there is no customer kiosk at the branch;
//! R-8 (`:30`) only staff issue numbers, and the kiosk `confirm` needs `QueueOperate` (D19 `:235`). Today the row is
//! `Need::Public` (`api/crates/xc-auth/src/domain/policy/areas/bookings.rs:24-28`), so anyone holding a booking QR
//! gets a number. After: `Need::Permission(Permission::QueueOperate)`, the same permission R-8 gives `confirm`.
//!
//! Who holds `QueueOperate` (seed `xc-auth/src/domain/access/role_grants.rs:144-260`: it is the Customers module at
//! Create, `permission_module.rs:110-114`): EXECUTIVE (Customers F), RATE_MANAGER (F), BRANCH_MANAGER (F) and
//! TELLER (C). Not ADMIN, FINANCE (Customers V), VERIFICATION_REVIEWER (V), MARKETING or CUSTOMER.
//!
//! The refusals, as the seam answers a permission-gated `Rpc` command (`Policy::authorize_with`,
//! `xc-auth/src/domain/policy/entities/policy.rs`; codes and statuses `policy_error.rs:103-155`):
//! - no session: 401 `signIn` (step 2, the `Rpc` path's sign-in sentence);
//! - a signed-in customer, even the booking's own: 400 `roleNotAllowedHere` (`Rpc`'s role refusal);
//! - a teller of another branch naming this one: 403 `outOfScope`;
//! - FINANCE of this branch (signed in, in scope, no `QueueOperate`): 400 `roleNotAllowedHere`.
//!
//! Every refusal leaves the booking CONFIRMED with no queue ticket. A success records the scanning staff member as
//! the `CHECK_IN` event's actor, not `"kiosk"` (controller claude/491239fc, 2026-10-27). The QR is the booking's
//! (`qr_token_hash` = SHA-256 of it), as `bk6_check_in_test.rs` seeds it.
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

const URI: &str = "/api/kiosk/bookings/check-in";

/// A CONFIRMED booking of Alice's at `B01`, in a slot starting in five minutes (inside the check-in window), whose
/// QR is `qr_token`.
async fn checkable_booking(db: &TestDb, id: &str, no: &str, qr_token: &str) {
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;
    seed_booking(
        &db.pool,
        id,
        no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &Sha256::digest(qr_token.as_bytes()),
    )
    .await;
}

async fn check_in(db: &TestDb, who: Option<&str>, qr_token: &str) -> Answer {
    let key = unique("k-gapsa9");
    call(
        db,
        who,
        Method::POST,
        URI,
        Some(json!({ "branch": B01, "qrToken": qr_token })),
        Some(&key),
    )
    .await
}

async fn status_of(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the stored booking")
}

async fn booking_tickets(db: &TestDb, id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM queue_tickets WHERE booking_id = $1 AND service_type = 'BOOKING'",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("count the booking's tickets")
}

/// `status` + `code`, and the booking untouched: still CONFIRMED, no ticket.
async fn refused(db: &TestDb, res: &Answer, id: &str, status: StatusCode, code: &str, what: &str) {
    assert_eq!(
        (res.status, res.body["error"]["code"].as_str()),
        (status, Some(code)),
        "{what}: expected {status} {code}; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(
        status_of(db, id).await,
        "CONFIRMED",
        "{what}: not checked in"
    );
    assert_eq!(booking_tickets(db, id).await, 0, "{what}: no queue number");
}

#[actix_web::test]
async fn gapsa9_no_session_is_refused_with_sign_in() {
    let db = TestDb::start().await;
    let id = "BK-gapsa9-anon";
    let qr = "gapsa9-anon-qr";
    checkable_booking(&db, id, "B261027-9001", qr).await;

    let res = check_in(&db, None, qr).await;
    refused(
        &db,
        &res,
        id,
        StatusCode::UNAUTHORIZED,
        "signIn",
        "no session, holding the booking's QR",
    )
    .await;

    db.finish().await;
}

#[actix_web::test]
async fn gapsa9_a_signed_in_customer_is_refused_even_for_their_own_booking() {
    let db = TestDb::start().await;
    let id = "BK-gapsa9-customer";
    let qr = "gapsa9-customer-qr";
    checkable_booking(&db, id, "B261027-9002", qr).await;

    let res = check_in(&db, Some("CUSTOMER@customer-alice"), qr).await;
    refused(
        &db,
        &res,
        id,
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "the booking's own customer",
    )
    .await;

    db.finish().await;
}

#[actix_web::test]
async fn gapsa9_a_teller_of_another_branch_is_refused_out_of_scope() {
    let db = TestDb::start().await;
    let id = "BK-gapsa9-other-branch";
    let qr = "gapsa9-other-branch-qr";
    checkable_booking(&db, id, "B261027-9003", qr).await;

    let res = check_in(&db, Some("TELLER@XC-B02"), qr).await;
    refused(
        &db,
        &res,
        id,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "a TELLER of XC-B02 naming XC-B01",
    )
    .await;

    db.finish().await;
}

#[actix_web::test]
async fn gapsa9_staff_of_the_branch_without_queue_operate_is_refused() {
    let db = TestDb::start().await;
    let id = "BK-gapsa9-finance";
    let qr = "gapsa9-finance-qr";
    checkable_booking(&db, id, "B261027-9004", qr).await;

    let res = check_in(&db, Some("FINANCE@XC-B01"), qr).await;
    refused(
        &db,
        &res,
        id,
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        "FINANCE of XC-B01 (no QueueOperate)",
    )
    .await;

    db.finish().await;
}

#[actix_web::test]
async fn gapsa9_the_qr_alone_is_refused_and_queue_staff_of_the_branch_check_the_booking_in() {
    let db = TestDb::start().await;

    // The customer's QR, with nobody signed in: refused, as the queue machine without its staff sign-in would be.
    let id = "BK-gapsa9-teller";
    let qr = "gapsa9-teller-qr";
    checkable_booking(&db, id, "B261027-9005", qr).await;
    let anon = check_in(&db, None, qr).await;
    refused(
        &db,
        &anon,
        id,
        StatusCode::UNAUTHORIZED,
        "signIn",
        "the QR alone",
    )
    .await;

    // The same QR, scanned by a TELLER of the branch: today's success, a BOOKING number.
    let res = check_in(&db, Some("TELLER@XC-B01"), qr).await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "a TELLER of XC-B01 checks the booking in; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(res.body["status"], "CHECKED_IN", "{}", res.text);
    assert!(
        res.body["ticketNumber"]
            .as_str()
            .is_some_and(|n| !n.is_empty()),
        "the answer carries the queue number: {}",
        res.text
    );
    assert_eq!(status_of(&db, id).await, "CHECKED_IN");
    assert_eq!(booking_tickets(&db, id).await, 1, "one BOOKING ticket");

    // A BRANCH_MANAGER of the branch holds QueueOperate too.
    let id_bm = "BK-gapsa9-manager";
    let qr_bm = "gapsa9-manager-qr";
    checkable_booking(&db, id_bm, "B261027-9006", qr_bm).await;
    let res_bm = check_in(&db, Some("BRANCH_MANAGER@XC-B01"), qr_bm).await;
    assert_eq!(
        res_bm.status,
        StatusCode::OK,
        "a BRANCH_MANAGER of XC-B01 checks the booking in; got {} {}",
        res_bm.status,
        res_bm.text
    );
    assert_eq!(status_of(&db, id_bm).await, "CHECKED_IN");
    assert_eq!(booking_tickets(&db, id_bm).await, 1, "one BOOKING ticket");

    db.finish().await;
}

/// The `CHECK_IN` event's `actor_id` (`booking_events`, written by `transition_in` from `check_in_in`'s actor).
async fn check_in_actor(db: &TestDb, id: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT actor_id FROM booking_events WHERE booking_id = $1 AND kind = 'CHECK_IN' ORDER BY at",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .expect("the booking's CHECK_IN events")
}

/// The check-in names the staff member who scanned, not `"kiosk"`: the desk check-in records `who.user_id`
/// (`commands/bookings/check_in.rs`), and now only staff reach this route (R-8), so its event says who issued the number.
#[actix_web::test]
async fn gapsa9_the_check_in_event_names_the_staff_member_who_scanned() {
    let db = TestDb::start().await;
    let id = "BK-gapsa9-actor";
    let qr = "gapsa9-actor-qr";
    checkable_booking(&db, id, "B261027-9007", qr).await;

    let res = check_in(&db, Some("TELLER@XC-B01"), qr).await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "a TELLER of XC-B01 checks the booking in; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(
        check_in_actor(&db, id).await,
        vec![TELLER_ID.to_string()],
        "one CHECK_IN event, whose actor is the teller's user id"
    );

    db.finish().await;
}
