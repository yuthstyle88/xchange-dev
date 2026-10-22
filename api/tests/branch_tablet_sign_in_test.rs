//! BT-5 (design `docs/design/blueprint/kiosk-shared-phone.md` §7.3 T-D7; §7.7 `otp/verify`;
//! §7.8 rows T14, T23; §7.10 Q-T1 A): the sign-in on a paired tablet binds the new customer
//! session to the tablet, replacing the one bound before; that session lasts at most 30 minutes
//! and `POST /api/auth/refresh` refuses it. A sign-in with no tablet cookie answers exactly as
//! before. Through the whole route table; Identity is the fake over real HTTP.
//!
//! Written before the implementation by the BE lane's skeleton (BT-0). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The tablet is written straight into `branch_tablets` (§7.6): its token is stored as the
//! SHA-256 of its whole text, so nothing here depends on the pairing routes (BT-4).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use chrono::{DateTime, Duration, Utc};
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_customer::application::consent_service::{ConsentService, Registration};
use xc_customer::domain::consent::Purpose;

const PHONE: &str = "+66812345678";
const CHALLENGE: &str = "c0ffee00-0000-4000-8000-0000000000b5";
const TABLET: &str = "7ab1e700-0000-4000-8000-000000000001";
const TABLET_TOKEN: &str = "xcbt_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const TABLET_COOKIE: &str = "__Host-xc_tablet";

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

    /// A paired tablet of MAIN holding `TABLET_TOKEN`; `revoked` sets `revoked_at`.
    async fn tablet(&self, revoked: bool) {
        sqlx::query(
            "INSERT INTO branch_tablets (id, branch_id, name, created_at, created_by, token_hash, paired_at, \
             revoked_at) VALUES ($1::uuid, 'MAIN', 'front tablet', now(), 'admin', $2, now(), \
             CASE WHEN $3 THEN now() END)",
        )
        .bind(TABLET)
        .bind(sha256_hex(TABLET_TOKEN))
        .bind(revoked)
        .execute(self.pool())
        .await
        .expect("a paired tablet (§7.6)");
    }

    /// A known customer of `subject` (as CI-6 registers one): its `users.id`.
    async fn customer(&self, subject: &str, name: &str) -> String {
        let user = bound_user(self.pool(), subject, "ACTIVE", name, &[("CUSTOMER", None)]).await;
        ConsentService::pg(self.pool().clone())
            .register(
                &self.org,
                Registration {
                    user_id: user.clone(),
                    display_name: name.to_owned(),
                    lang: "th".to_owned(),
                    marketing: vec![],
                    evidence_ref: "test".to_owned(),
                },
                Utc::now(),
            )
            .await
            .expect("CI-6 register");
        user
    }

    /// Identity's verify answers a login with `access_token` for a known person.
    fn verify_answers(&self, access_token: &str) {
        let mut login = login_body(access_token, REFRESH_1);
        let o = login.as_object_mut().unwrap();
        o.insert("phoneVerified".to_owned(), json!(true));
        o.insert("phone".to_owned(), json!(PHONE));
        self.identity.answer(
            "/auth/otp/verify",
            200,
            &json!({ "verified": true, "login": login, "registered": false }).to_string(),
        );
    }

    /// `POST /api/auth/otp/verify` carrying `cookie` (the tablet's, or none).
    async fn verify(&self, cookie: Option<&str>) -> Reply {
        let mut req = Req::post(
            "/api/auth/otp/verify",
            json!({ "challengeId": CHALLENGE, "code": "123456",
                    "register": { "displayName": "Somchai J.", "lang": "th", "marketing": [] } }),
        );
        req.cookie = cookie.map(str::to_owned);
        self.app.send(req).await
    }

    async fn signed_in(&self, cookie: Option<&str>) -> (String, Reply) {
        let reply = self.verify(cookie).await;
        assert_eq!(reply.status, 200, "{}", reply.text);
        (reply.session(), reply)
    }

    /// `(customer_session_hash, customer_user_id)` of the tablet.
    async fn bound(&self) -> (Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT customer_session_hash, customer_user_id::text FROM branch_tablets WHERE id = $1::uuid",
        )
        .bind(TABLET)
        .fetch_one(self.pool())
        .await
        .expect("read the tablet")
    }

    async fn session_exists(&self, token: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_sessions WHERE hash = $1")
            .bind(sha256_hex(token))
            .fetch_one(self.pool())
            .await
            .expect("count")
            == 1
    }

    async fn expires(&self, token: &str) -> DateTime<Utc> {
        sqlx::query_scalar("SELECT expires FROM user_sessions WHERE hash = $1")
            .bind(sha256_hex(token))
            .fetch_one(self.pool())
            .await
            .expect("read the session")
    }

    async fn finish(self) {
        self.desk.drop_schema().await;
    }
}

fn tablet_cookie() -> String {
    format!("{TABLET_COOKIE}={TABLET_TOKEN}")
}

fn cookie_names(reply: &Reply) -> Vec<String> {
    let mut names: Vec<String> = reply.cookies.keys().cloned().collect();
    names.sort();
    names
}

/// T14: a sign-in carrying a live tablet's cookie binds the new session (its hash, its user) to
/// the tablet; the next sign-in on it replaces the binding and deletes the session bound before.
#[actix_web::test]
async fn t14_a_sign_in_on_a_paired_tablet_binds_the_session_and_replaces_the_last_one() {
    let shop = Shop::start().await;
    shop.tablet(false).await;
    let first_user = shop.customer(SUB_A, "Nok").await;
    let second_user = shop.customer(SUB_B, "Lek").await;

    shop.verify_answers(VALID_A);
    let (first, _) = shop.signed_in(Some(&tablet_cookie())).await;
    assert_eq!(
        shop.bound().await,
        (Some(sha256_hex(&first)), Some(first_user))
    );

    shop.verify_answers(VALID_B);
    let (second, _) = shop.signed_in(Some(&tablet_cookie())).await;
    assert_eq!(
        shop.bound().await,
        (Some(sha256_hex(&second)), Some(second_user))
    );
    assert!(
        !shop.session_exists(&first).await,
        "the session bound before is deleted"
    );
    assert!(shop.session_exists(&second).await);
    shop.finish().await;
}

/// A sign-in with no tablet cookie, or with a revoked tablet's, binds nothing and answers exactly
/// as one with no cookie: the same body, the same cookies, a 12-hour session.
#[actix_web::test]
async fn a_sign_in_without_a_live_tablet_answers_as_before_and_binds_nothing() {
    let shop = Shop::start().await;
    shop.tablet(true).await;
    shop.customer(SUB_A, "Nok").await;
    shop.verify_answers(VALID_A);

    let (plain, plain_reply) = shop.signed_in(None).await;
    let (revoked, revoked_reply) = shop.signed_in(Some(&tablet_cookie())).await;
    assert_eq!(
        revoked_reply.text, plain_reply.text,
        "the same body, byte for byte"
    );
    assert_eq!(cookie_names(&revoked_reply), cookie_names(&plain_reply));
    assert!(
        !revoked_reply.cookies.contains_key(TABLET_COOKIE),
        "verify never writes the tablet cookie"
    );
    assert_eq!(revoked_reply.cache_control, plain_reply.cache_control);
    assert_eq!(
        shop.bound().await,
        (None, None),
        "a revoked tablet binds nothing"
    );
    for token in [&plain, &revoked] {
        let expires = shop.expires(token).await;
        assert!(
            expires > Utc::now() + Duration::hours(11),
            "12 hours: {expires}"
        );
    }
    shop.finish().await;
}

/// T23 (Q-T1 A): the tablet-bound session expires at most 30 minutes after sign-in.
#[actix_web::test]
async fn t23_a_tablet_bound_session_lasts_at_most_thirty_minutes() {
    let shop = Shop::start().await;
    shop.tablet(false).await;
    shop.customer(SUB_A, "Nok").await;
    shop.verify_answers(VALID_A);
    let before = Utc::now();
    let (token, _) = shop.signed_in(Some(&tablet_cookie())).await;
    let expires = shop.expires(&token).await;
    assert!(
        expires <= Utc::now() + Duration::minutes(30) && expires >= before + Duration::minutes(29),
        "sign-in + 30 min: {expires}"
    );
    shop.finish().await;
}

/// T23: `POST /api/auth/refresh` refuses a tablet-bound session with `401` and never extends it
/// (Identity is not asked); a session not bound to a tablet refreshes as today.
#[actix_web::test]
async fn t23_refresh_refuses_a_tablet_bound_session_and_refreshes_any_other() {
    let shop = Shop::start().await;
    shop.tablet(false).await;
    shop.customer(SUB_A, "Nok").await;
    shop.verify_answers(VALID_A);
    let (bound, _) = shop.signed_in(Some(&tablet_cookie())).await;
    let (phone, _) = shop.signed_in(None).await;
    let capped = shop.expires(&bound).await;
    shop.identity
        .login_answers("/auth/refresh", VALID_A, REFRESH_2);

    let refused = shop
        .app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&bound))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    assert!(
        shop.identity.received("/auth/refresh").is_empty(),
        "Identity is not asked"
    );
    if shop.session_exists(&bound).await {
        assert_eq!(shop.expires(&bound).await, capped, "never extended");
    }

    let refreshed = shop
        .app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&phone))
        .await;
    assert_eq!(refreshed.status, 200, "{}", refreshed.text);
    assert_eq!(shop.identity.received("/auth/refresh").len(), 1);
    let body: Value = refreshed.json();
    assert!(body["user"].is_object(), "{}", refreshed.text);
    shop.finish().await;
}
