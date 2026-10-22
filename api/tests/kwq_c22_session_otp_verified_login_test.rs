//! KWQ-C2-2 — `SessionService::otp_verified_login` (design `kiosk-shared-phone.md` §4.2 D11, §4.4 B14–B15, §4.5):
//! Identity's verify with the sign-up flag passed through unchanged, the access token checked by the verifier,
//! `Ok(None)` for a confirmed code with no account, and nobody signed in. The Identity client is an in-process fake
//! that records the flag; the verifier is the real JWKS verifier fed by the fake Identity's JWKS.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::sync::{Arc, Mutex};

use auth_http_common::Desk;
use identity_signin_common::*;
use xc_auth::application::session::{SessionError, SessionService};
use xc_auth::domain::identity::identity_client::{
    IdentityClient, IdentityError, IdentityLoginResponse, IdentityOtpRequestResponse,
    IdentityOtpVerifyResponse, IdentityRegisterRequest, IdentityRegisterResponse,
};
use xc_auth::domain::token::TokenVerifier;
use xc_auth::infrastructure::token::http_jwks_source::HttpJwksSource;
use xc_auth::infrastructure::token::jwks_verifier::JwksTokenVerifier;

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Answers every `otp_verify_registering` with `answer`, recording (challenge, code, flag).
struct Fake {
    answer: Result<IdentityOtpVerifyResponse, IdentityError>,
    calls: Mutex<Vec<(String, String, bool)>>,
}

impl Fake {
    fn new(answer: Result<IdentityOtpVerifyResponse, IdentityError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> Vec<(String, String, bool)> {
        self.calls.lock().unwrap().clone()
    }
}

impl IdentityClient for Fake {
    fn login<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>> {
        unreachable!("no password login here")
    }
    fn refresh<'a>(
        &'a self,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityLoginResponse, IdentityError>> {
        unreachable!()
    }
    fn logout<'a>(
        &'a self,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>> {
        unreachable!()
    }
    fn otp_request<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpRequestResponse, IdentityError>> {
        unreachable!()
    }
    fn otp_verify<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpVerifyResponse, IdentityError>> {
        unreachable!("the customer door calls otp_verify_registering")
    }
    fn otp_verify_registering<'a>(
        &'a self,
        challenge_id: &'a str,
        code: &'a str,
        register_if_absent: bool,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdentityOtpVerifyResponse, IdentityError>> {
        self.calls.lock().unwrap().push((
            challenge_id.to_owned(),
            code.to_owned(),
            register_if_absent,
        ));
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
    fn register<'a>(
        &'a self,
        _: &'a IdentityRegisterRequest<'a>,
    ) -> BoxFuture<'a, Result<IdentityRegisterResponse, IdentityError>> {
        unreachable!()
    }
    fn change_password<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a str,
        _: Option<&'a str>,
    ) -> BoxFuture<'a, Result<(), IdentityError>> {
        unreachable!()
    }
}

fn login(token: &str) -> IdentityLoginResponse {
    serde_json::from_value(login_body(token, REFRESH_1)).expect("a login answer")
}

fn session(desk: &Desk, identity: &FakeIdentity, fake: Arc<Fake>) -> SessionService {
    let verifier: Arc<dyn TokenVerifier> = Arc::new(JwksTokenVerifier::new(
        Arc::new(HttpJwksSource::new(format!(
            "{}/.well-known/jwks.json",
            identity.url()
        ))),
        ISSUER,
        AUDIENCE,
    ));
    SessionService::new(
        desk.pool.clone(),
        Some(fake as Arc<dyn IdentityClient>),
        Some(verifier),
        Some(CLIENT_SECRET.to_owned()),
    )
}

#[actix_web::test]
async fn a_login_is_verified_the_flag_passes_through_and_nobody_is_signed_in() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    for register in [true, false] {
        let fake = Fake::new(Ok(IdentityOtpVerifyResponse {
            verified: true,
            login: Some(login(VALID_A)),
            registered: Some(register),
        }));
        let s = session(&desk, &identity, fake.clone());
        let got = s
            .otp_verified_login("chal-1", "123456", register, Some(BROWSER_ADDRESS))
            .await
            .expect("verified")
            .expect("a login");
        assert_eq!(got.verified.subject, SUB_A);
        assert_eq!(got.registered, register);
        assert_eq!(got.login.access_token, VALID_A);
        assert_eq!(
            fake.calls(),
            vec![("chal-1".to_owned(), "123456".to_owned(), register)]
        );
    }
    assert_eq!(all_sessions(&desk.pool).await, 0, "no session is made");
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_confirmed_code_with_no_account_is_none() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let fake = Fake::new(Ok(IdentityOtpVerifyResponse {
        verified: true,
        login: None,
        registered: None,
    }));
    let got = session(&desk, &identity, fake)
        .otp_verified_login("chal-1", "123456", false, None)
        .await
        .expect("no error");
    assert!(got.is_none());
    desk.drop_schema().await;
}

#[actix_web::test]
async fn identitys_refusal_and_a_bad_token_are_errors() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let refused = session(&desk, &identity, Fake::new(Err(IdentityError::InvalidCode)))
        .otp_verified_login("chal-1", "000000", true, None)
        .await;
    assert_eq!(
        refused.map(|_| ()),
        Err(SessionError::Identity(IdentityError::InvalidCode))
    );

    let bad = Fake::new(Ok(IdentityOtpVerifyResponse {
        verified: true,
        login: Some(login(BAD_SIG_A)),
        registered: None,
    }));
    let refused = session(&desk, &identity, bad)
        .otp_verified_login("chal-1", "123456", false, None)
        .await;
    assert!(
        matches!(
            refused,
            Err(SessionError::Identity(IdentityError::Unavailable { .. }))
        ),
        "a token the verifier refuses is 503, as verified_login answers it: {:?}",
        refused.map(|_| ())
    );
    desk.drop_schema().await;
}
