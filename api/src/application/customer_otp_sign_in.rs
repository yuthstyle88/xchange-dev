//! `CustomerOtpSignIn` (design `kiosk-shared-phone.md` §4.2 D11, §4.4 B14–B15): the customer door of
//! `POST /api/auth/otp/verify`, by SMS or e-mail OTP, with first-sight sign-up.
//!
//! 1. `SessionService::otp_verified_login(challenge, code, register.is_some(), ip)`:
//!    `Ok(None)` (Identity confirmed the code, no account, no sign-up asked) ⇒ `NotRegistered`.
//! 2. The subject already has a `users` row ⇒ signed in as today (`sign_in_with_login`), whatever `register` says:
//!    `registered: false`, `customer_id` = that user's customer if any (B14 "known").
//! 3. No `users` row and no `register` ⇒ `NotRegistered` (B15). [proposed] This replaces today's `401 signIn`
//!    ("no user in XCHANGE") on the OTP door only; the password door keeps it.
//! 4. No `users` row and `register` ⇒ in one transaction: the organisation (`signup_organization`), then
//!    `CustomerBinder::bind_in` with the proven contacts (`proven_contacts`), `evidence_ref = "web:otp"`,
//!    `created_by = "customer-register"`; commit; then `sign_in_with_login`. `registered: true`, `customer_id` = the
//!    bound customer (new, or joined: B14 "on a desk customer without a user"). The binder's refusals pass through.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use xc_auth::application::session::{SessionError, SessionService, SignInTail};
use xc_auth::domain::identity::identity_client::IdentityLoginResponse;
use xc_customer::domain::consent::Channel;

use crate::application::customer_binder::{BindError, ProvenContact};
use crate::application::otp_challenge_seal::OtpChallengeSeal;

/// `register: {displayName, lang, marketing}` on the verify body, already checked for shape.
#[derive(Debug, Clone)]
pub struct OtpRegister {
    pub display_name: String,
    pub lang: String,
    pub marketing: Vec<Channel>,
}

/// A signed-in customer door answer.
pub struct OtpSignedIn {
    pub tail: SignInTail,
    /// The user's customer: the one bound now, or the one an existing user already has (`None` for staff).
    pub customer_id: Option<String>,
    /// A customer was bound in this call (new or joined).
    pub registered: bool,
}

#[derive(Debug)]
pub enum OtpSignInError {
    Session(SessionError),
    /// `401 notRegistered`: the code is right but there is no customer to sign in, and no sign-up was asked.
    NotRegistered,
    Bind(BindError),
}

impl From<SessionError> for OtpSignInError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

pub struct CustomerOtpSignIn<'a> {
    session: &'a SessionService,
    pool: &'a PgPool,
}

impl<'a> CustomerOtpSignIn<'a> {
    pub fn new(session: &'a SessionService, pool: &'a PgPool) -> Self {
        Self { session, pool }
    }

    /// Steps 1–4 of the module doc. `seal` is the opened `__Host-xc_otp` cookie, if any.
    pub async fn sign_in(
        &self,
        challenge_id: &str,
        code: &str,
        seal: Option<&OtpChallengeSeal>,
        register: Option<OtpRegister>,
        client_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<OtpSignedIn, OtpSignInError> {
        let _ = (
            challenge_id,
            code,
            seal,
            register,
            client_ip,
            now,
            self.session,
            self.pool,
        );
        todo!("KWQ-C2-5: the customer OTP door")
    }
}

/// The contacts this sign-in proved: the PHONE when Identity's login has `phone` with `phoneVerified: true` (and it
/// normalises); the EMAIL when `seal` is for `challenge_id` and its channel is `Email`. A seal for another challenge
/// proves nothing. Never trusts a destination from the request body.
pub fn proven_contacts(
    login: &IdentityLoginResponse,
    seal: Option<&OtpChallengeSeal>,
    challenge_id: &str,
) -> Vec<ProvenContact> {
    let _ = (login, seal, challenge_id);
    todo!("KWQ-C2-5: the proven contacts")
}
