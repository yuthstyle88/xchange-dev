//! KWQ-D5 — `BookingCopy` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D21) against a
//! real PostgreSQL: the member's own booking, found by `qr_token_hash` (the raw SHA-256 of the
//! QR), with its number, date, slot, currency, amount, branch and live status — and never a
//! passport, a phone or a customer number. Another member's booking, an unknown id and a
//! reissued-away QR find nothing.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Harness: `bk6_common` (branch `XC-B01`, customers Alice and Bob). Needs `TEST_DATABASE_URL`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use bk6_common::*;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xchange::application::copy_by_email::booking_copy::BookingCopy;
use xchange::application::copy_by_email::subject::{CopySubject, CopySubjectKind};

fn sha(qr: &str) -> [u8; 32] {
    Sha256::digest(qr.as_bytes()).into()
}

async fn seeded(db: &TestDb) {
    let slot = Uuid::new_v4();
    let start = Utc::now() + Duration::hours(3);
    seed_slot(&db.pool, slot, B01, bangkok_today(), start, start + Duration::minutes(30), 5, 1, "OPEN").await;
    seed_booking(&db.pool, "BK-ALICE", "B261022-0001", CUSTOMER_ALICE_CUST_ID, B01, slot, "APP", "CONFIRMED", &sha("alice-qr")).await;
    seed_booking(&db.pool, "BK-BOB", "B261022-0002", CUSTOMER_BOB_CUST_ID, B01, slot, "APP", "CONFIRMED", &sha("bob-qr")).await;
}

fn row<'a>(rows: &'a [(String, String)], field: &str) -> Option<&'a str> {
    rows.iter().find(|(f, _)| f == field).map(|(_, v)| v.as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_members_booking_is_found_by_its_qr_with_its_details_and_status() {
    let db = TestDb::start().await;
    seeded(&db).await;
    let subject = BookingCopy::new(db.pool.clone());
    assert_eq!(subject.kind(), CopySubjectKind::Booking);

    let d = subject
        .load_for(CUSTOMER_ALICE_CUST_ID, "BK-ALICE", &sha("alice-qr"))
        .await
        .expect("read")
        .expect("Alice's booking");
    assert_eq!((d.kind, d.id.as_str()), (CopySubjectKind::Booking, "BK-ALICE"));
    assert_eq!(d.branch_name, "Branch 1");
    assert_eq!(d.status, "CONFIRMED");
    assert_eq!(row(&d.rows, "bookingNo"), Some("B261022-0001"));
    assert_eq!(row(&d.rows, "date"), Some(bangkok_today().format("%Y-%m-%d").to_string().as_str()));
    assert!(row(&d.rows, "slot").is_some_and(|s| s.contains(':')), "{:?}", d.rows);
    assert_eq!(row(&d.rows, "currency"), Some("USD"));
    assert!(row(&d.rows, "amount").is_some_and(|a| a.starts_with("100")), "{:?}", d.rows);
    let all = format!("{d:?}");
    for secret in ["CUST-ALICE", "CN-001", "passport", "+66"] {
        assert!(!all.contains(secret), "{secret} must never be in a copy: {all}");
    }

    let by_link = subject
        .load_by_link("BK-ALICE", &sha("alice-qr"))
        .await
        .expect("read")
        .expect("by link");
    assert_eq!(by_link, d);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn another_members_booking_an_unknown_id_or_another_qr_finds_nothing() {
    let db = TestDb::start().await;
    seeded(&db).await;
    let subject = BookingCopy::new(db.pool.clone());
    let none = |r: pos_error::AppResult<Option<_>>| r.expect("read").is_none();

    assert!(none(subject.load_for(CUSTOMER_ALICE_CUST_ID, "BK-BOB", &sha("bob-qr")).await));
    assert!(none(subject.load_for(CUSTOMER_ALICE_CUST_ID, "BK-NOPE", &sha("alice-qr")).await));
    assert!(none(subject.load_for(CUSTOMER_ALICE_CUST_ID, "BK-ALICE", &sha("old-qr")).await));
    assert!(none(subject.load_by_link("BK-ALICE", &sha("bob-qr")).await));

    // a reissue replaces the hash: the old QR finds nothing, the new one does
    sqlx::query("UPDATE bookings SET qr_token_hash = $2, qr_version = 2 WHERE id = $1")
        .bind("BK-ALICE")
        .bind(&sha("alice-qr-2")[..])
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(none(subject.load_for(CUSTOMER_ALICE_CUST_ID, "BK-ALICE", &sha("alice-qr")).await));
    assert!(none(subject.load_by_link("BK-ALICE", &sha("alice-qr")).await));
    assert!(
        subject
            .load_by_link("BK-ALICE", &sha("alice-qr-2"))
            .await
            .expect("read")
            .is_some()
    );
    db.finish().await;
}
