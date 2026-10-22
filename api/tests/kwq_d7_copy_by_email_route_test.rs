//! KWQ-D7 — `POST /api/app/copy-by-email` (`copyByEmail`, design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D21, §4.4 B19–B23), through the whole route
//! table on a real PostgreSQL: the refusals a member gets, in the core's order (subject → verified
//! e-mail → sender), the validation of the body, and the policy (`CustomerSelf`).
//! The send itself is pinned by KWQ-D3 with a fake sender; here no Notify is configured: these
//! tests assume `NOTIFY_BASE_URL` / `NOTIFY_API_KEY` are unset in the test process (as in CI).
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Harness: `bk6_common`. Needs `TEST_DATABASE_URL`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use bk6_common::*;
use chrono::{Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xc_kernel::pii::{PiiCipher, PiiKeys};

const ROUTE: &str = "/api/app/copy-by-email";
const ALICE: &str = "CUSTOMER@customer-alice";

fn sha(qr: &str) -> [u8; 32] {
    Sha256::digest(qr.as_bytes()).into()
}

async fn alices_booking(db: &TestDb) {
    let slot = Uuid::new_v4();
    let start = Utc::now() + Duration::hours(3);
    seed_slot(&db.pool, slot, B01, bangkok_today(), start, start + Duration::minutes(30), 5, 1, "OPEN").await;
    seed_booking(&db.pool, "BK-ALICE", "B261022-0001", CUSTOMER_ALICE_CUST_ID, B01, slot, "APP", "CONFIRMED", &sha("alice-qr")).await;
}

async fn alices_email(db: &TestDb) {
    let keys = Arc::new(PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap());
    let sealed = PiiCipher::new(keys).encrypt(format!("{CUSTOMER_ALICE_CUST_ID}EMAIL").as_bytes(), b"alice@example.com");
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, \
         key_id, lookup_hash, verified_at, status) \
         SELECT gen_random_uuid(), organization_id, id, 'EMAIL', $2, $3, $4, now(), 'ACTIVE' \
         FROM customers WHERE id = $1",
    )
    .bind(CUSTOMER_ALICE_CUST_ID)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&[9u8; 32][..])
    .execute(&db.pool)
    .await
    .expect("seed Alice's verified e-mail");
}

async fn copy(db: &TestDb, who: Option<&str>, body: serde_json::Value) -> Answer {
    call(db, who, Method::POST, ROUTE, Some(body), None).await
}

fn code(a: &Answer) -> &str {
    a.body["error"]["code"].as_str().unwrap_or("")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_refusals_come_in_the_cores_order() {
    let db = TestDb::start().await;
    alices_booking(&db).await;

    // B21: an unknown id, or a QR that is not the current one
    for body in [
        json!({ "subject": "booking", "id": "BK-NOPE", "qr": "alice-qr" }),
        json!({ "subject": "booking", "id": "BK-ALICE", "qr": "old-qr" }),
        json!({ "subject": "webRequest", "id": "BK-ALICE", "qr": "alice-qr" }),
    ] {
        let a = copy(&db, Some(ALICE), body.clone()).await;
        assert_eq!((a.status, code(&a)), (StatusCode::NOT_FOUND, "notFound"), "{body}: {}", a.text);
    }

    // B22: the right QR, but no verified e-mail yet
    let a = copy(&db, Some(ALICE), json!({ "subject": "booking", "id": "BK-ALICE", "qr": "alice-qr" })).await;
    assert_eq!((a.status, code(&a)), (StatusCode::CONFLICT, "noVerifiedEmail"), "{}", a.text);

    // B23: with a verified e-mail and no Notify configured
    alices_email(&db).await;
    let a = copy(&db, Some(ALICE), json!({ "subject": "booking", "id": "BK-ALICE", "qr": "alice-qr" })).await;
    assert_eq!(
        (a.status, code(&a)),
        (StatusCode::SERVICE_UNAVAILABLE, "emailUnavailable"),
        "{}",
        a.text
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_body_is_validation_failed_naming_the_field() {
    let db = TestDb::start().await;
    for (body, field) in [
        (json!({ "subject": "parcel", "id": "BK-1", "qr": "q" }), "subject"),
        (json!({ "id": "BK-1", "qr": "q" }), "subject"),
        (json!({ "subject": "booking", "qr": "q" }), "id"),
        (json!({ "subject": "booking", "id": "", "qr": "q" }), "id"),
        (json!({ "subject": "booking", "id": "BK-1" }), "qr"),
        (json!({ "subject": "booking", "id": "BK-1", "qr": "" }), "qr"),
    ] {
        let a = copy(&db, Some(ALICE), body.clone()).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{body}: {}", a.text);
        assert_eq!(code(&a), "validationFailed", "{body}: {}", a.text);
        assert_eq!(a.body["error"]["field"], json!(field), "{body}: {}", a.text);
    }
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_signed_in_member_may_ask() {
    let db = TestDb::start().await;
    alices_booking(&db).await;
    let body = json!({ "subject": "booking", "id": "BK-ALICE", "qr": "alice-qr" });

    let anonymous = copy(&db, None, body.clone()).await;
    assert!(anonymous.status.is_client_error(), "{}", anonymous.text);
    assert_ne!(code(&anonymous), "noVerifiedEmail", "{}", anonymous.text);

    let teller = copy(&db, Some(&format!("TELLER@{B01}")), body).await;
    assert!(teller.status.is_client_error(), "{}", teller.text);
    assert_ne!(code(&teller), "noVerifiedEmail", "a teller never reaches the core: {}", teller.text);
    db.finish().await;
}
