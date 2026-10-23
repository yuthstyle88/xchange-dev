//! KWQ-C2-4 — `CustomerBinder::bind_in` (design `kiosk-shared-phone.md` §4.2 D11, §4.3; owner Q1; §4.4 B14) against
//! a real PostgreSQL: a new person becomes a new customer with each proven contact as an ACTIVE encrypted identity;
//! a contact ACTIVE on a desk-made customer with no user joins that customer; a contact on a customer that has a
//! user is refused and nothing is written. The rules are the module doc of `api/src/application/customer_binder.rs`.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::sync::Arc;

use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::{SUB_A, SUB_B, bound_user, organization};
use sqlx::{AssertSqlSafe, PgPool};
use xc_customer::application::consent_service::ConsentService;
use xc_customer::domain::consent::{Channel, Purpose};
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, Sealed, normalize};
use xchange::application::customer_binder::{
    BindError, BindInput, Bound, CustomerBinder, ProvenContact,
};

const PHONE: &str = "+66812345678";
const EMAIL: &str = "somchai@example.com";

fn keys() -> Arc<PiiKeys> {
    xc_customer::get_pii_keys().expect("the test PII key is installed")
}

fn hash(t: IdentityType, raw: &str) -> Vec<u8> {
    let v = normalize(t, raw, None).expect("normalises");
    LookupHasher::new(&keys()).hash(t, &v).to_vec()
}

struct Shop {
    desk: Desk,
    org: String,
}

impl Shop {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let desk = Desk::start().await;
        let org = organization(&desk.pool).await;
        for purpose in [Purpose::Terms, Purpose::Marketing] {
            ConsentService::pg(desk.pool.clone())
                .publish(&org, None, purpose, "th", "ข้อความยินยอม", Utc::now())
                .await
                .expect("publish");
        }
        Self { desk, org }
    }

    fn pool(&self) -> &PgPool {
        &self.desk.pool
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(self.pool())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// (users, customers, consents, ACTIVE identities)
    async fn written(&self) -> (i64, i64, i64, i64) {
        (
            self.count("SELECT count(*) FROM users").await,
            self.count("SELECT count(*) FROM customers").await,
            self.count("SELECT count(*) FROM consents").await,
            self.count("SELECT count(*) FROM customer_identities WHERE status = 'ACTIVE'")
                .await,
        )
    }

    /// A customer made at the desk: no user, one ACTIVE identity of `t` = `raw`.
    async fn desk_customer(&self, id: &str, t: IdentityType, raw: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, preferred_language, \
                 status, mode, user_id, created_at) \
             VALUES ($1, $2::uuid, $1, 'Desk Name', 'en', 'ACTIVE', 'LIVE', NULL, now())",
        )
        .bind(id)
        .bind(&self.org)
        .execute(self.pool())
        .await
        .expect("a desk customer");
        self.identity(id, t, raw).await;
    }

    async fn identity(&self, customer: &str, t: IdentityType, raw: &str) {
        let v = normalize(t, raw, None).expect("normalises");
        let sealed = PiiCipher::new(keys())
            .encrypt(format!("{customer}{}", t.as_str()).as_bytes(), v.as_bytes());
        sqlx::query(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
                 lookup_hash, verified_at, status) \
             VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, $5, $6, now(), 'ACTIVE')",
        )
        .bind(&self.org)
        .bind(customer)
        .bind(t.as_str())
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(hash(t, raw))
        .execute(self.pool())
        .await
        .expect("an identity");
    }

    /// A customer that already has a user (CI-6), with one ACTIVE identity of `t` = `raw`.
    async fn customer_with_user(&self, subject: &str, t: IdentityType, raw: &str) -> String {
        let user = bound_user(
            self.pool(),
            subject,
            "ACTIVE",
            "Other",
            &[("CUSTOMER", None)],
        )
        .await;
        let customer = ConsentService::pg(self.pool().clone())
            .register(
                &self.org,
                xc_customer::application::consent_service::Registration {
                    user_id: user,
                    display_name: "Other".to_owned(),
                    lang: "th".to_owned(),
                    marketing: vec![],
                    evidence_ref: "test".to_owned(),
                },
                Utc::now(),
            )
            .await
            .expect("CI-6 register");
        self.identity(&customer, t, raw).await;
        customer
    }

    async fn bind(
        &self,
        subject: &str,
        proven: &[ProvenContact],
        marketing: Vec<Channel>,
        lang: &str,
    ) -> Result<Bound, BindError> {
        let mut tx = self.pool().begin().await.expect("begin");
        let out = CustomerBinder::bind_in(
            &mut tx,
            &self.org,
            subject,
            proven,
            BindInput {
                display_name: "Somchai J.".to_owned(),
                lang: lang.to_owned(),
                marketing,
                evidence_ref: "web:otp".to_owned(),
                created_by: "customer-register".to_owned(),
            },
            Utc::now(),
        )
        .await;
        if out.is_ok() {
            tx.commit().await.expect("commit");
        }
        out
    }

    async fn finish(self) {
        self.desk.drop_schema().await;
    }
}

fn phone() -> ProvenContact {
    ProvenContact::Phone(normalize(IdentityType::Phone, PHONE, None).unwrap())
}

fn email() -> ProvenContact {
    ProvenContact::Email(normalize(IdentityType::Email, EMAIL, None).unwrap())
}

#[actix_web::test]
async fn a_new_person_becomes_a_new_customer_with_each_proven_contact() {
    let shop = Shop::start().await;
    let bound = shop
        .bind(SUB_A, &[phone(), email()], vec![Channel::Sms], "th")
        .await
        .expect("bound");
    assert!(!bound.joined);
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM customers WHERE id = '{}' AND user_id = '{}'::uuid",
            bound.customer_id, bound.user_id
        ))
        .await,
        1
    );
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{}'::uuid AND auth_subject = '{SUB_A}' \
             AND status = 'ACTIVE' AND created_by = 'customer-register'",
            bound.user_id
        ))
        .await,
        1
    );
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM user_roles WHERE user_id = '{}'::uuid AND role_code = 'CUSTOMER' \
             AND branch_id IS NULL",
            bound.user_id
        ))
        .await,
        1
    );
    // TERMS + one MARKETING (SMS), evidence web:otp
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM consents WHERE customer_id = '{}' AND evidence_ref = 'web:otp'",
            bound.customer_id
        ))
        .await,
        2
    );
    for (t, raw) in [(IdentityType::Phone, PHONE), (IdentityType::Email, EMAIL)] {
        let (bytes, key_id): (Vec<u8>, String) = sqlx::query_as(
            "SELECT encrypted_value, key_id FROM customer_identities \
             WHERE customer_id = $1 AND type = $2 AND status = 'ACTIVE' AND verified_at IS NOT NULL \
               AND lookup_hash = $3",
        )
        .bind(&bound.customer_id)
        .bind(t.as_str())
        .bind(hash(t, raw))
        .fetch_one(shop.pool())
        .await
        .unwrap_or_else(|e| panic!("one ACTIVE {} identity: {e}", t.as_str()));
        let plain = PiiCipher::new(keys())
            .decrypt(
                format!("{}{}", bound.customer_id, t.as_str()).as_bytes(),
                &Sealed { key_id, bytes },
            )
            .expect("AAD customer_id ‖ TYPE");
        assert_eq!(
            String::from_utf8(plain).unwrap(),
            normalize(t, raw, None).unwrap()
        );
    }
    shop.finish().await;
}

#[actix_web::test]
async fn a_contact_on_a_desk_customer_without_a_user_joins_that_customer() {
    for (t, raw, proof) in [
        (IdentityType::Phone, PHONE, phone()),
        (IdentityType::Email, EMAIL, email()),
    ] {
        let shop = Shop::start().await;
        shop.desk_customer("CX-DESK0001", t, raw).await;
        let before = shop.written().await;
        let bound = shop
            .bind(SUB_A, &[proof], vec![], "th")
            .await
            .expect("joined");
        assert!(bound.joined, "{}", t.as_str());
        assert_eq!(bound.customer_id, "CX-DESK0001");
        let after = shop.written().await;
        assert_eq!(after.0, before.0 + 1, "one new user");
        assert_eq!(after.1, before.1, "no new customer");
        assert_eq!(after.3, before.3, "the identity is not written twice");
        assert_eq!(
            shop.count(&format!(
                "SELECT count(*) FROM customers WHERE id = 'CX-DESK0001' AND user_id = '{}'::uuid \
                 AND display_name = 'Desk Name' AND preferred_language = 'en'",
                bound.user_id
            ))
            .await,
            1,
            "the desk customer now has the user; its name and language are the desk's"
        );
        assert_eq!(
            shop.count(
                "SELECT count(*) FROM consents WHERE customer_id = 'CX-DESK0001' \
                 AND purpose = 'TERMS' AND granted AND evidence_ref = 'web:otp'"
            )
            .await,
            1,
            "TERMS is recorded on the joined customer"
        );
        shop.finish().await;
    }
}

#[actix_web::test]
async fn a_contact_on_a_customer_with_a_user_is_refused_and_writes_nothing() {
    for (t, raw, proof, refusal) in [
        (IdentityType::Phone, PHONE, phone(), BindError::PhoneInUse),
        (IdentityType::Email, EMAIL, email(), BindError::EmailInUse),
    ] {
        let shop = Shop::start().await;
        shop.customer_with_user(SUB_B, t, raw).await;
        let before = shop.written().await;
        assert_eq!(shop.bind(SUB_A, &[proof], vec![], "th").await, Err(refusal));
        assert_eq!(
            shop.written().await,
            before,
            "{}: nothing written",
            t.as_str()
        );
        shop.finish().await;
    }
}

#[actix_web::test]
async fn a_subject_that_already_has_a_user_is_already_registered() {
    let shop = Shop::start().await;
    bound_user(
        shop.pool(),
        SUB_A,
        "ACTIVE",
        "Staff",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let before = shop.written().await;
    assert_eq!(
        shop.bind(SUB_A, &[phone()], vec![], "th").await,
        Err(BindError::AlreadyRegistered)
    );
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

#[actix_web::test]
async fn no_terms_in_the_language_is_consent_text_missing_and_writes_nothing() {
    let shop = Shop::start().await;
    let before = shop.written().await;
    assert_eq!(
        shop.bind(SUB_A, &[phone()], vec![], "zh").await,
        Err(BindError::ConsentTextMissing)
    );
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

#[actix_web::test]
async fn no_proven_contact_still_makes_a_customer() {
    let shop = Shop::start().await;
    let bound = shop.bind(SUB_A, &[], vec![], "th").await.expect("bound");
    assert!(!bound.joined);
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM customer_identities WHERE customer_id = '{}'",
            bound.customer_id
        ))
        .await,
        0
    );
    shop.finish().await;
}
