//! KWQ-C2-1 — `HttpIdentityClient::otp_verify_registering` (design `kiosk-shared-phone.md` §4.2 D11, §4.5): the
//! verify call carries Identity's first-sight sign-up flag `registerIfAbsent` exactly as asked, and reads Identity's
//! `registered` answer. Against the fake Identity over real HTTP (`identity_signin_common::FakeIdentity`).
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken;
//! add tests in a file of your own.

mod common;
mod identity_signin_common;

use identity_signin_common::*;
use serde_json::json;
use xc_auth::domain::identity::identity_client::{IdentityClient, IdentityError};
use xc_auth::infrastructure::identity::HttpIdentityClient;

fn client(identity: &FakeIdentity) -> HttpIdentityClient {
    let env = identity_env(&identity.url());
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let settings = common::settings(&pairs);
    HttpIdentityClient::new(
        settings.identity.clone().expect("identity settings"),
        "x-real-ip",
    )
}

#[actix_web::test]
async fn the_flag_is_sent_as_asked_and_registered_is_read() {
    let identity = FakeIdentity::start();
    identity.answer(
        "/auth/otp/verify",
        200,
        &json!({ "verified": true, "login": login_body(VALID_A, REFRESH_1), "registered": true })
            .to_string(),
    );
    let c = client(&identity);

    let yes = c
        .otp_verify_registering("chal-1", "123456", true, Some(BROWSER_ADDRESS))
        .await
        .expect("verified");
    assert_eq!(yes.registered, Some(true));
    assert!(yes.login.is_some());
    let no = c
        .otp_verify_registering("chal-2", "654321", false, Some(BROWSER_ADDRESS))
        .await
        .expect("verified");
    assert!(no.login.is_some());

    let sent = identity.received("/auth/otp/verify");
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[0].json(),
        json!({ "challengeId": "chal-1", "code": "123456", "registerIfAbsent": true })
    );
    assert_eq!(
        sent[1].json(),
        json!({ "challengeId": "chal-2", "code": "654321", "registerIfAbsent": false })
    );
}

#[actix_web::test]
async fn a_confirmed_code_with_no_account_answers_no_login() {
    let identity = FakeIdentity::start();
    identity.answer("/auth/otp/verify", 200, r#"{"verified":true}"#);
    let answer = client(&identity)
        .otp_verify_registering("chal-1", "123456", false, None)
        .await
        .expect("verified");
    assert!(answer.login.is_none());
    assert_ne!(answer.registered, Some(true));
}

#[actix_web::test]
async fn identitys_refusal_maps_as_otp_verify_maps_it() {
    let identity = FakeIdentity::start();
    identity.answer(
        "/auth/otp/verify",
        401,
        r#"{"error":{"code":"invalid_code","message":"x"}}"#,
    );
    let c = client(&identity);
    let registering = c
        .otp_verify_registering("chal-1", "000000", true, None)
        .await
        .map(|_| ());
    let plain = c.otp_verify("chal-1", "000000", None).await.map(|_| ());
    assert_eq!(registering, plain, "the same error mapping as otp_verify");
    assert_eq!(registering, Err(IdentityError::InvalidCode));
}
