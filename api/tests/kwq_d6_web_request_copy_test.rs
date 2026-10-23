//! KWQ-D6 — `WebRequestCopy` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D2, D4,
//! D21; W5) against a real PostgreSQL: the member's own web request, found by its request token
//! hash (`cancel_token_hash`, lowercase hex SHA-256), with branch, currency, amount, rate, expiry
//! (`created_at` + 60 min) and its W5 status — never a phone, passport or customer number.
//! Another member's request, a kiosk request (no member), an unknown id and a reissued-away
//! token find nothing; a purged request has no row.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! ⚠️ Needs `kiosk_requests.customer_id`, added by slice a's migration
//! (`20261219220000_kiosk_requests_customer.sql`, on `feat/kwq-a`). On a branch without it, the
//! seed below fails ("column customer_id does not exist"); release KWQ-D6 after slice a is on main
//! and `feat/kwq-d` has merged main.
//!
//! Harness: `bk6_common` (branch `XC-B01`, customers Alice and Bob). Needs `TEST_DATABASE_URL`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use bk6_common::*;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use xchange::application::copy_by_email::subject::{CopySubject, CopySubjectKind};
use xchange::application::copy_by_email::web_request_copy::WebRequestCopy;

fn sha(qr: &str) -> [u8; 32] {
    Sha256::digest(qr.as_bytes()).into()
}

/// A request of `XC-B01` in `status`, created `created`, with `member` and the hash of `token`.
async fn request(
    db: &TestDb,
    id: &str,
    status: &str,
    member: Option<&str>,
    token: Option<&str>,
    created: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, \
             foreign_amount, thb, input_amount, payment_method, customer_id, cancel_token_hash) \
         VALUES ($1, $2, $3, $4, 'BUY', 'USD', 35.5, 100.00, 3550.00, 100.00, 'Cash', $5, \
             CASE WHEN $6::text IS NULL THEN NULL ELSE encode(sha256(convert_to($6, 'UTF8')), 'hex') END)",
    )
    .bind(B01)
    .bind(id)
    .bind(status)
    .bind(created)
    .bind(member)
    .bind(token)
    .execute(&db.pool)
    .await
    .expect("seed a request (needs slice a's kiosk_requests.customer_id)");
}

fn row<'a>(rows: &'a [(String, String)], field: &str) -> Option<&'a str> {
    rows.iter()
        .find(|(f, _)| f == field)
        .map(|(_, v)| v.as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_members_web_request_is_found_by_its_token_with_its_details() {
    let db = TestDb::start().await;
    let created = Utc::now() - Duration::minutes(5);
    request(
        &db,
        "WR-ALICE",
        "QUOTED",
        Some(CUSTOMER_ALICE_CUST_ID),
        Some("tok-a"),
        created,
    )
    .await;
    request(
        &db,
        "WR-WAIT",
        "WAITING",
        Some(CUSTOMER_ALICE_CUST_ID),
        Some("tok-w"),
        created,
    )
    .await;
    let subject = WebRequestCopy::new(db.pool.clone());
    assert_eq!(subject.kind(), CopySubjectKind::WebRequest);

    let d = subject
        .load_for(CUSTOMER_ALICE_CUST_ID, "WR-ALICE", &sha("tok-a"))
        .await
        .expect("read")
        .expect("Alice's request");
    assert_eq!(
        (d.kind, d.id.as_str()),
        (CopySubjectKind::WebRequest, "WR-ALICE")
    );
    assert_eq!(d.branch_name, "Branch 1");
    assert_eq!(d.status, "QUOTED", "W5: no ticket yet");
    assert_eq!(row(&d.rows, "currency"), Some("USD"));
    assert!(
        row(&d.rows, "amount").is_some_and(|a| a.starts_with("100")),
        "{:?}",
        d.rows
    );
    assert!(
        row(&d.rows, "rate").is_some_and(|r| r.starts_with("35.5")),
        "{:?}",
        d.rows
    );
    let expires = DateTime::parse_from_rfc3339(row(&d.rows, "expiresAt").expect("expiresAt"))
        .expect("RFC 3339")
        .with_timezone(&Utc);
    assert!(
        (expires - (created + Duration::minutes(60)))
            .num_seconds()
            .abs()
            <= 1
    );
    let all = format!("{d:?}");
    for secret in ["CUST-ALICE", "CN-001", "tok-a", "+66"] {
        assert!(
            !all.contains(secret),
            "{secret} must never be in a copy: {all}"
        );
    }
    assert_eq!(
        subject
            .load_by_link("WR-ALICE", &sha("tok-a"))
            .await
            .expect("read"),
        Some(d)
    );

    let waiting = subject
        .load_for(CUSTOMER_ALICE_CUST_ID, "WR-WAIT", &sha("tok-w"))
        .await
        .expect("read")
        .expect("found");
    assert_eq!(waiting.status, "WAITING");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn another_member_a_kiosk_request_an_unknown_id_or_another_token_finds_nothing() {
    let db = TestDb::start().await;
    let now = Utc::now();
    request(
        &db,
        "WR-BOB",
        "QUOTED",
        Some(CUSTOMER_BOB_CUST_ID),
        Some("tok-b"),
        now,
    )
    .await;
    request(&db, "KIOSK-1", "WAITING", None, Some("tok-k"), now).await;
    let subject = WebRequestCopy::new(db.pool.clone());
    let none = |r: pos_error::AppResult<Option<_>>| r.expect("read").is_none();

    assert!(none(
        subject
            .load_for(CUSTOMER_ALICE_CUST_ID, "WR-BOB", &sha("tok-b"))
            .await
    ));
    assert!(none(
        subject
            .load_for(CUSTOMER_ALICE_CUST_ID, "KIOSK-1", &sha("tok-k"))
            .await
    ));
    assert!(none(
        subject
            .load_for(CUSTOMER_BOB_CUST_ID, "WR-NOPE", &sha("tok-b"))
            .await
    ));
    assert!(none(
        subject
            .load_for(CUSTOMER_BOB_CUST_ID, "WR-BOB", &sha("tok-x"))
            .await
    ));
    assert!(none(subject.load_by_link("WR-BOB", &sha("tok-x")).await));
    assert!(
        none(subject.load_by_link("WR-GONE", &sha("tok-b")).await),
        "a purged request has no row"
    );
    db.finish().await;
}
