//! KWQ-D9 — `my_profile`'s `emailCopy` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.3a
//! W11): `emailCopy: {available, to}`; `available` = a verified, ACTIVE `EMAIL` identity exists
//! **and** the Notify client is configured; `to` = that address masked (`a***@example.com`), `null`
//! when there is none. Without Notify configured (the test process, as in CI) `available` is
//! `false` while `to` still shows the masked address.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Harness: `bk6_common`. Needs `TEST_DATABASE_URL`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use bk6_common::*;
use serde_json::json;
use xc_kernel::pii::{PiiCipher, PiiKeys};

async fn email(db: &TestDb, customer: &str, address: &str, status: &str, n: u8) {
    let keys =
        Arc::new(PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap());
    let sealed =
        PiiCipher::new(keys).encrypt(format!("{customer}EMAIL").as_bytes(), address.as_bytes());
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, \
         key_id, lookup_hash, verified_at, status) \
         SELECT gen_random_uuid(), organization_id, id, 'EMAIL', $2, $3, $4, now(), $5 \
         FROM customers WHERE id = $1",
    )
    .bind(customer)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&[n; 32][..])
    .bind(status)
    .execute(&db.pool)
    .await
    .expect("seed an e-mail");
}

async fn profile(db: &TestDb, who: &str) -> serde_json::Value {
    let a = call(
        db,
        Some(who),
        Method::GET,
        "/api/v3/views/my_profile",
        None,
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    a.body["data"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_with_a_verified_email_sees_it_masked_and_unavailable_without_notify() {
    let db = TestDb::start().await;
    email(
        &db,
        CUSTOMER_ALICE_CUST_ID,
        "alice@example.com",
        "ACTIVE",
        1,
    )
    .await;
    let data = profile(&db, "CUSTOMER@customer-alice").await;
    assert_eq!(
        data["emailCopy"],
        json!({ "available": false, "to": "a***@example.com" }),
        "{data}"
    );
    assert!(
        !data.to_string().contains("alice@example.com"),
        "never the clear address: {data}"
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_without_one_has_no_address_and_no_copy() {
    let db = TestDb::start().await;
    email(&db, CUSTOMER_BOB_CUST_ID, "bob@example.com", "REVOKED", 2).await;
    let data = profile(&db, "CUSTOMER@customer-bob").await;
    assert_eq!(
        data["emailCopy"],
        json!({ "available": false, "to": null }),
        "{data}"
    );
    db.finish().await;
}
