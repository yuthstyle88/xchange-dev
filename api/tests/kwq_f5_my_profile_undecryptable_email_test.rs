//! KWQ-F5 (#4713) — when the stored verified e-mail cannot be decrypted under the active PII keys,
//! `my_profile` answers `200` with `emailCopy: {available: false, to: null}` and the rest of the
//! profile intact, rather than `500`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use bk6_common::*;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
async fn an_undecryptable_stored_email_degrades_email_copy_to_null_without_failing_the_profile() {
    let db = TestDb::start().await;
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, \
         key_id, lookup_hash, verified_at, status) \
         SELECT gen_random_uuid(), organization_id, id, 'EMAIL', $2, 'test-1', $3, now(), 'ACTIVE' \
         FROM customers WHERE id = $1",
    )
    .bind(CUSTOMER_ALICE_CUST_ID)
    .bind(&b"not-valid-ciphertext"[..])
    .bind(&[9u8; 32][..])
    .execute(&db.pool)
    .await
    .expect("seed undecryptable e-mail");

    let a = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        "/api/v3/views/my_profile",
        None,
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let data = &a.body["data"];
    assert_eq!(data["customerId"], json!(CUSTOMER_ALICE_CUST_ID), "{data}");
    assert_eq!(data["customerNo"], json!("CN-001"), "{data}");
    assert_eq!(data["displayName"], json!("Alice"), "{data}");
    assert_eq!(data["status"], json!("ACTIVE"), "{data}");
    assert!(data["consents"].is_array(), "{data}");
    assert_eq!(
        data["emailCopy"],
        json!({ "available": false, "to": null }),
        "{data}"
    );
    db.finish().await;
}
