//! KWQ-C2-6 — `/api/app/register` through `CustomerBinder` (design `kiosk-shared-phone.md` §4.2 D11, §4.3:
//! "`register_customer` splits into a login half and a shared `CustomerBinder`"; owner Q1). The app register now
//! joins a desk-made customer whose ACTIVE PHONE is the verified phone, exactly as the OTP door does; everything
//! else answers as before (`customer_account_test.rs` keeps pinning it).
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::*;
use serde_json::json;
use sqlx::AssertSqlSafe;
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::Purpose;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, normalize};

const PHONE: &str = "+66812345678";

#[actix_web::test]
async fn the_app_register_joins_a_desk_customer_with_the_verified_phone() {
    common::install_test_pii_keys();
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let org = organization(&desk.pool).await;
    ConsentService::pg(desk.pool.clone())
        .publish(&org, None, Purpose::Terms, "th", "ข้อความยินยอม", Utc::now())
        .await
        .expect("publish TERMS");

    let keys = xc_customer::get_pii_keys().expect("keys");
    let e164 = normalize(IdentityType::Phone, PHONE, None).unwrap();
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, preferred_language, \
             status, mode, user_id, created_at) \
         VALUES ('CX-DESK0001', $1::uuid, 'CX-DESK0001', 'Desk Name', 'th', 'ACTIVE', 'LIVE', NULL, now())",
    )
    .bind(&org)
    .execute(&desk.pool)
    .await
    .expect("a desk customer");
    let sealed = PiiCipher::new(keys.clone()).encrypt(b"CX-DESK0001PHONE", e164.as_bytes());
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status) \
         VALUES (gen_random_uuid(), $1::uuid, 'CX-DESK0001', 'PHONE', $2, $3, $4, now(), 'ACTIVE')",
    )
    .bind(&org)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(LookupHasher::new(&keys).hash(IdentityType::Phone, &e164).to_vec())
    .execute(&desk.pool)
    .await
    .expect("its PHONE");

    let reply = app
        .send(Req::post(
            "/api/app/register",
            json!({ "identifier": "somchai@example.com", "password": "pw-from-the-app-0001",
                    "displayName": "Somchai J.", "lang": "th", "marketing": [] }),
        ))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["customerId"], json!("CX-DESK0001"));
    reply.session();

    let count = |sql: &'static str| {
        let pool = desk.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
                .fetch_one(&pool)
                .await
                .expect(sql)
        }
    };
    assert_eq!(count("SELECT count(*) FROM customers").await, 1, "no second customer");
    assert_eq!(
        count("SELECT count(*) FROM customers WHERE id = 'CX-DESK0001' AND user_id IS NOT NULL").await,
        1
    );
    assert_eq!(
        count("SELECT count(*) FROM customer_identities WHERE type = 'PHONE' AND status = 'ACTIVE'").await,
        1,
        "the PHONE is not written twice"
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM consents WHERE customer_id = 'CX-DESK0001' AND purpose = 'TERMS' \
             AND evidence_ref = 'app:register'"
        )
        .await,
        1
    );
    desk.drop_schema().await;
}
