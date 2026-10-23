//! KWQ-C2-5 — the customer OTP door through the whole route table (design `kiosk-shared-phone.md` §4.2 D11, §4.4
//! B14–B15): `POST /api/auth/otp/request` seals the challenge's destination into `__Host-xc_otp`, and
//! `POST /api/auth/otp/verify` runs `CustomerOtpSignIn`: first-sight sign-up with `register`, joining a desk-made
//! customer, `409 phoneInUse`, and `401 notRegistered` without `register`. Identity is the fake over real HTTP.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! `[required]` wire, beyond today's (`identity_signin_test.rs` keeps pinning today's):
//! - `otp/request {destination, channel: "sms"|"email"}`: the same `200 {challengeId, expiresAt}`, plus
//!   `Set-Cookie: __Host-xc_otp=<seal>; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=<= 600`. Any other
//!   `channel` ⇒ `400 invalidRequest`, Identity not asked.
//! - `otp/verify {challengeId, code, register?: {displayName, lang, marketing}}` ⇒ `200 {user, customerId,
//!   registered}` with the session cookie; `__Host-xc_otp` cleared (`Max-Age=0`) on every answer. Refusals:
//!   `401 notRegistered`; `409 phoneInUse` / `emailInUse`; `409 consentTextMissing`; `400 validationFailed` naming
//!   `lang`, `displayName` or `marketing` (Identity not asked), as `/api/app/register` answers them.
//! - The destination is never taken from the verify body.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::sync::Arc;

use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};
use xc_customer::application::consent_service::{ConsentService, Registration};
use xc_customer::domain::consent::Purpose;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys, normalize};
use xchange::application::otp_challenge_seal::{OTP_COOKIE, OtpChallengeSeal, OtpChannel};

const PHONE: &str = "+66812345678";
const EMAIL: &str = "somchai@example.com";
const CHALLENGE: &str = "c0ffee00-0000-4000-8000-000000000001";

fn keys() -> Arc<PiiKeys> {
    xc_customer::get_pii_keys().expect("the test PII key is installed")
}

fn hash(t: IdentityType, raw: &str) -> Vec<u8> {
    let v = normalize(t, raw, None).expect("normalises");
    LookupHasher::new(&keys()).hash(t, &v).to_vec()
}

struct Shop {
    desk: Desk,
    identity: FakeIdentity,
    app: App,
    org: String,
}

impl Shop {
    async fn start() -> Self {
        common::install_test_pii_keys();
        let desk = Desk::start().await;
        let identity = FakeIdentity::start();
        let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
        let org = organization(&desk.pool).await;
        ConsentService::pg(desk.pool.clone())
            .publish(&org, None, Purpose::Terms, "th", "ข้อความยินยอม", Utc::now())
            .await
            .expect("publish TERMS");
        identity.answer(
            "/auth/otp/request",
            200,
            &json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 }).to_string(),
        );
        Self {
            desk,
            identity,
            app,
            org,
        }
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

    /// (users, customers, consents, ACTIVE identities, sessions)
    async fn written(&self) -> (i64, i64, i64, i64, i64) {
        (
            self.count("SELECT count(*) FROM users").await,
            self.count("SELECT count(*) FROM customers").await,
            self.count("SELECT count(*) FROM consents").await,
            self.count("SELECT count(*) FROM customer_identities WHERE status = 'ACTIVE'")
                .await,
            self.count("SELECT count(*) FROM user_sessions").await,
        )
    }

    /// Identity's verify answers a login for `SUB_A` with `phone` verified (or no phone), `registered` as given.
    fn verify_answers(&self, phone: Option<&str>, registered: bool) {
        let mut login = login_body(VALID_A, REFRESH_1);
        let o = login.as_object_mut().unwrap();
        o.remove("phone");
        o.insert("phoneVerified".to_owned(), json!(phone.is_some()));
        if let Some(p) = phone {
            o.insert("phone".to_owned(), json!(p));
        }
        self.identity.answer(
            "/auth/otp/verify",
            200,
            &json!({ "verified": true, "login": login, "registered": registered }).to_string(),
        );
    }

    async fn request(&self, destination: &str, channel: &str) -> Reply {
        self.app
            .send(Req::post(
                "/api/auth/otp/request",
                json!({ "destination": destination, "channel": channel }),
            ))
            .await
    }

    async fn verify(&self, body: Value, seal: Option<&str>) -> Reply {
        let mut req = Req::post("/api/auth/otp/verify", body);
        req.cookie = seal.map(|s| format!("{OTP_COOKIE}={s}"));
        self.app.send(req).await
    }

    async fn desk_customer(&self, id: &str, t: IdentityType, raw: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, preferred_language, \
                 status, mode, user_id, created_at) \
             VALUES ($1, $2::uuid, $1, 'Desk Name', 'th', 'ACTIVE', 'LIVE', NULL, now())",
        )
        .bind(id)
        .bind(&self.org)
        .execute(self.pool())
        .await
        .expect("a desk customer");
        self.identity_row(id, t, raw).await;
    }

    async fn identity_row(&self, customer: &str, t: IdentityType, raw: &str) {
        let v = normalize(t, raw, None).unwrap();
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

    async fn customer_of_subject(&self, subject: &str, name: &str) -> String {
        let user = bound_user(self.pool(), subject, "ACTIVE", name, &[("CUSTOMER", None)]).await;
        ConsentService::pg(self.pool().clone())
            .register(
                &self.org,
                Registration {
                    user_id: user,
                    display_name: name.to_owned(),
                    lang: "th".to_owned(),
                    marketing: vec![],
                    evidence_ref: "test".to_owned(),
                },
                Utc::now(),
            )
            .await
            .expect("CI-6 register")
    }

    async fn finish(self) {
        self.desk.drop_schema().await;
    }
}

fn register() -> Value {
    json!({ "displayName": "Somchai J.", "lang": "th", "marketing": [] })
}

fn verify_body(with_register: bool) -> Value {
    let mut b = json!({ "challengeId": CHALLENGE, "code": "123456" });
    if with_register {
        b["register"] = register();
    }
    b
}

fn cleared_seal(reply: &Reply) {
    let c = reply
        .cookies
        .get(OTP_COOKIE)
        .unwrap_or_else(|| panic!("{OTP_COOKIE} is cleared: {:?}", reply.cookies));
    assert_eq!(c.max_age, 0, "{OTP_COOKIE} cleared: {c:?}");
}

#[actix_web::test]
async fn the_request_seals_the_destination_into_an_http_only_cookie() {
    let shop = Shop::start().await;
    let reply = shop.request(PHONE, "sms").await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(
        reply.json(),
        json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 })
    );
    let cookie = reply.cookies.get(OTP_COOKIE).expect("the seal cookie");
    assert_eq!(
        cookie.attributes,
        "Path=/; HttpOnly; Secure; SameSite=Strict"
    );
    assert!(cookie.max_age > 0 && cookie.max_age <= 600, "{cookie:?}");
    assert_eq!(
        OtpChallengeSeal::open(&cookie.value, &keys()),
        Some(OtpChallengeSeal {
            channel: OtpChannel::Sms,
            destination: normalize(IdentityType::Phone, PHONE, None).unwrap(),
            challenge_id: CHALLENGE.to_owned(),
        })
    );

    let fax = shop.request(PHONE, "fax").await;
    assert_eq!(fax.status, 400, "{}", fax.text);
    assert_eq!(fax.refusal().0, "invalidRequest");
    assert_eq!(
        shop.identity.received("/auth/otp/request").len(),
        1,
        "fax never reached Identity"
    );
    shop.finish().await;
}

/// B14 new, SMS: Identity signs the new person up; one user, one customer, one PHONE identity; registered.
#[actix_web::test]
async fn a_new_phone_with_register_signs_up_and_signs_in() {
    let shop = Shop::start().await;
    let seal = shop.request(PHONE, "sms").await.cookies[OTP_COOKIE]
        .value
        .clone();
    shop.verify_answers(Some(PHONE), true);
    let reply = shop.verify(verify_body(true), Some(&seal)).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = reply.json();
    assert_eq!(body["registered"], json!(true));
    let customer = body["customerId"].as_str().expect("customerId").to_owned();
    reply.session();
    cleared_seal(&reply);
    assert_eq!(
        shop.identity.received("/auth/otp/verify")[0].json()["registerIfAbsent"],
        json!(true)
    );
    assert_eq!(
        shop.count(&format!(
            "SELECT count(*) FROM customer_identities WHERE customer_id = '{customer}' AND type = 'PHONE' \
             AND status = 'ACTIVE'"
        ))
        .await,
        1
    );
    shop.finish().await;
}

/// B14 new, e-mail: the address comes from the seal, never from the body.
#[actix_web::test]
async fn an_email_sign_up_binds_the_sealed_address_and_ignores_one_in_the_body() {
    let shop = Shop::start().await;
    let seal = shop.request(EMAIL, "email").await.cookies[OTP_COOKIE]
        .value
        .clone();
    shop.verify_answers(None, true);
    let mut body = verify_body(true);
    body["destination"] = json!("someone-else@example.com");
    let reply = shop.verify(body, Some(&seal)).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    let customer = reply.json()["customerId"].as_str().unwrap().to_owned();
    let emails: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT lookup_hash FROM customer_identities WHERE customer_id = $1 AND type = 'EMAIL' \
         AND status = 'ACTIVE'",
    )
    .bind(&customer)
    .fetch_all(shop.pool())
    .await
    .unwrap();
    assert_eq!(emails, vec![hash(IdentityType::Email, EMAIL)]);
    shop.finish().await;
}

/// Without the seal (or with one for another challenge) no e-mail is bound; the sign-up still succeeds.
#[actix_web::test]
async fn without_a_matching_seal_no_email_is_bound() {
    let shop = Shop::start().await;
    let other = OtpChallengeSeal {
        channel: OtpChannel::Email,
        destination: normalize(IdentityType::Email, EMAIL, None).unwrap(),
        challenge_id: "another-challenge".to_owned(),
    }
    .seal(&keys());
    shop.verify_answers(None, true);
    let reply = shop.verify(verify_body(true), Some(&other)).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(
        shop.count("SELECT count(*) FROM customer_identities WHERE type = 'EMAIL'")
            .await,
        0
    );
    shop.finish().await;
}

/// B14 known: a user that already exists is signed in; nothing is bound; `registered: false`.
#[actix_web::test]
async fn a_known_customer_is_signed_in_with_its_customer_id() {
    let shop = Shop::start().await;
    let customer = shop.customer_of_subject(SUB_A, "Nok").await;
    shop.verify_answers(Some(PHONE), false);
    let before = shop.written().await;
    let reply = shop.verify(verify_body(true), None).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["registered"], json!(false));
    assert_eq!(reply.json()["customerId"], json!(customer));
    let after = shop.written().await;
    assert_eq!(
        (after.0, after.1, after.2, after.3),
        (before.0, before.1, before.2, before.3)
    );
    shop.finish().await;
}

/// B14 on a desk customer without a user: joined (owner Q1).
#[actix_web::test]
async fn a_phone_on_a_desk_customer_joins_it() {
    let shop = Shop::start().await;
    shop.desk_customer("CX-DESK0001", IdentityType::Phone, PHONE)
        .await;
    shop.verify_answers(Some(PHONE), true);
    let reply = shop.verify(verify_body(true), None).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["customerId"], json!("CX-DESK0001"));
    assert_eq!(reply.json()["registered"], json!(true));
    assert_eq!(
        shop.count(
            "SELECT count(*) FROM customers WHERE id = 'CX-DESK0001' AND user_id IS NOT NULL"
        )
        .await,
        1
    );
    shop.finish().await;
}

/// B14 with a user: `409 phoneInUse`, nothing written, no session.
#[actix_web::test]
async fn a_phone_on_a_customer_with_a_user_is_phone_in_use() {
    let shop = Shop::start().await;
    let other = shop.customer_of_subject(SUB_B, "Other").await;
    shop.identity_row(&other, IdentityType::Phone, PHONE).await;
    shop.verify_answers(Some(PHONE), true);
    let before = shop.written().await;
    let reply = shop.verify(verify_body(true), None).await;
    assert_eq!(reply.status, 409, "{}", reply.text);
    assert_eq!(reply.refusal().0, "phoneInUse");
    assert!(reply.sets_no_session());
    cleared_seal(&reply);
    assert_eq!(shop.written().await, before);
    shop.finish().await;
}

/// B15: without `register`, an unknown destination and an Identity account with no user here are both
/// `401 notRegistered`; Identity is asked with `registerIfAbsent: false`.
#[actix_web::test]
async fn without_register_an_unknown_person_is_not_registered() {
    let shop = Shop::start().await;
    shop.identity
        .answer("/auth/otp/verify", 200, r#"{"verified":true}"#);
    let reply = shop.verify(verify_body(false), None).await;
    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(reply.refusal().0, "notRegistered");
    assert!(reply.sets_no_session());

    shop.verify_answers(Some(PHONE), false);
    let reply = shop.verify(verify_body(false), None).await;
    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(reply.refusal().0, "notRegistered");

    for sent in shop.identity.received("/auth/otp/verify") {
        assert_eq!(sent.json()["registerIfAbsent"], json!(false));
    }
    assert_eq!(shop.written().await.0, 0, "no user");
    shop.finish().await;
}

#[actix_web::test]
async fn a_bad_register_is_validation_failed_before_identity() {
    let shop = Shop::start().await;
    for (register, field) in [
        (
            json!({ "displayName": "S", "lang": "xx", "marketing": [] }),
            "lang",
        ),
        (
            json!({ "displayName": "  ", "lang": "th", "marketing": [] }),
            "displayName",
        ),
        (
            json!({ "displayName": "S", "lang": "th", "marketing": ["FAX"] }),
            "marketing",
        ),
    ] {
        let mut body = verify_body(false);
        body["register"] = register;
        let reply = shop.verify(body, None).await;
        assert_eq!(reply.status, 400, "{field}: {}", reply.text);
        assert_eq!(reply.json()["error"]["code"], json!("validationFailed"));
        assert_eq!(reply.json()["error"]["field"], json!(field));
    }
    assert!(shop.identity.received("/auth/otp/verify").is_empty());
    shop.finish().await;
}
