//! KWQ-D4 — `PgMemberEmails` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D21, W11)
//! against a real PostgreSQL: the member's verified, ACTIVE `EMAIL` identity, decrypted with the
//! PII keys (AAD `<customer_id>EMAIL`), and the member's `preferred_language`. A revoked, an
//! unverified, a phone-only or another member's identity is never answered.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Harness: `bk6_common` (customers `CUST-ALICE-1`, `CUST-BOB-1`). Needs `TEST_DATABASE_URL`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use bk6_common::*;
use sqlx::PgPool;
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::application::copy_by_email::member_email::{
    MemberEmails, PgMemberEmails, VerifiedEmail,
};

fn keys() -> Arc<PiiKeys> {
    Arc::new(
        PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
            .expect("the test PII key parses"),
    )
}

/// An EMAIL identity of `customer`, sealed as the service seals it.
async fn email(pool: &PgPool, customer: &str, address: &str, status: &str, verified: bool, n: u8) {
    let sealed = PiiCipher::new(keys()).encrypt(format!("{customer}EMAIL").as_bytes(), address.as_bytes());
    let verified_at = if verified { "now() - interval '1 day'" } else { "NULL" };
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, \
         key_id, lookup_hash, verified_at, status) \
         SELECT gen_random_uuid(), organization_id, id, 'EMAIL', $2, $3, $4, {verified_at}, $5 \
         FROM customers WHERE id = $1"
    )))
    .bind(customer)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(&[n; 32][..])
    .bind(status)
    .execute(pool)
    .await
    .expect("seed an e-mail identity");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_verified_active_email_is_answered_decrypted_with_the_members_language() {
    let db = TestDb::start().await;
    sqlx::query("UPDATE customers SET preferred_language = 'en' WHERE id = $1")
        .bind(CUSTOMER_ALICE_CUST_ID)
        .execute(&db.pool)
        .await
        .unwrap();
    email(&db.pool, CUSTOMER_ALICE_CUST_ID, "alice@example.com", "ACTIVE", true, 1).await;

    let emails = PgMemberEmails::new(db.pool.clone(), keys());
    assert_eq!(
        emails.verified_email(CUSTOMER_ALICE_CUST_ID).await.expect("read"),
        Some(VerifiedEmail {
            address: "alice@example.com".to_owned(),
            lang: "en".to_owned(),
        })
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn revoked_unverified_phone_only_and_unknown_members_have_none() {
    let db = TestDb::start().await;
    email(&db.pool, CUSTOMER_BOB_CUST_ID, "bob-old@example.com", "REVOKED", true, 2).await;
    email(&db.pool, CUSTOMER_BOB_CUST_ID, "bob-new@example.com", "ACTIVE", false, 3).await;
    seed_customer_phone(&db.pool, CUSTOMER_ALICE_CUST_ID, "+66812345678").await;

    let emails = PgMemberEmails::new(db.pool.clone(), keys());
    assert_eq!(emails.verified_email(CUSTOMER_BOB_CUST_ID).await.expect("read"), None);
    assert_eq!(
        emails.verified_email(CUSTOMER_ALICE_CUST_ID).await.expect("read"),
        None,
        "a phone is not an e-mail"
    );
    assert_eq!(emails.verified_email("CUST-NOBODY").await.expect("read"), None);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn one_members_email_is_never_answered_for_another() {
    let db = TestDb::start().await;
    email(&db.pool, CUSTOMER_ALICE_CUST_ID, "alice@example.com", "ACTIVE", true, 4).await;
    let emails = PgMemberEmails::new(db.pool.clone(), keys());
    assert_eq!(emails.verified_email(CUSTOMER_BOB_CUST_ID).await.expect("read"), None);
    db.finish().await;
}
