//! Customer sign-up (#654 CI-7; design `docs/design/blueprint/customer-identity-consent.md` §4.4, K7, K8, K10;
//! the lane controller's rulings 2-4 of 2026-10-10 on #654 and the PHONE addition of 07:42Z).
//!
//! An Identity user becomes a customer in one transaction: their `users` row (bound to the Identity subject), an
//! organisation-wide CUSTOMER role, the customer with its TERMS (and MARKETING) consents, and, when Identity
//! reports a verified phone, one ACTIVE PHONE identity. Then they are signed in. Any refusal writes nothing.
//! No identity value, hash or secret is logged or put in an error.

use chrono::{DateTime, Utc};
use pos_error::AppError;
use sqlx::PgPool;
use xc_auth::application::session::{SessionError, SessionService, SignInTail};
use xc_auth::domain::identity::identity_client::IdentityLoginResponse;
use xc_customer::domain::consent::{Channel, ConsentError};
use xc_kernel::pii::{IdentityType, normalize};

use crate::application::customer_binder::{BindError, BindInput, CustomerBinder, ProvenContact};

/// What `POST /api/app/register` asks for, already checked for shape.
pub struct SignupInput {
    pub identifier: String,
    pub password: String,
    pub display_name: String,
    pub lang: String,
    pub marketing: Vec<Channel>,
}

/// Why a sign-up wrote nothing.
#[derive(Debug)]
pub enum SignupError {
    /// Identity refused the login or is unavailable, or signing in failed.
    Session(SessionError),
    /// The Identity subject already has a user here: a customer or staff (R3, R4, R13).
    AlreadyRegistered,
    /// No TERMS text (or MARKETING text, when channels are chosen) in the request's language (§4.3).
    ConsentTextMissing,
    /// The verified phone is ACTIVE on another customer of the organisation (07:42Z, #554's one-active index).
    PhoneInUse,
    Internal(String),
}

impl From<SessionError> for SignupError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

impl From<BindError> for SignupError {
    fn from(e: BindError) -> Self {
        match e {
            BindError::AlreadyRegistered => Self::AlreadyRegistered,
            BindError::ConsentTextMissing => Self::ConsentTextMissing,
            BindError::PhoneInUse => Self::PhoneInUse,
            BindError::EmailInUse => Self::Internal("unexpected email in use".to_owned()),
            BindError::Internal(m) => Self::Internal(m),
        }
    }
}

/// Who wrote a customer's own user and role, in `users.created_by` and `user_audit`.
const CREATED_BY: &str = "customer-register";
/// The consent evidence of a sign-up made in the app.
const EVIDENCE: &str = "app:register";

/// Whether `e` is `ConsentError::TextMissing`: `From<ConsentError> for AppError` keeps only its sentence, so the
/// one match on that sentence lives here (`consentTextMissing`, §4.3).
pub fn is_consent_text_missing(e: &AppError) -> bool {
    matches!(e, AppError::Domain(m) if *m == ConsentError::TextMissing.to_string())
}

fn internal(e: impl std::fmt::Display) -> SignupError {
    SignupError::Internal(e.to_string())
}

/// The organisation a customer signs up to: the install's one organisation, `code = 'XC'` (ruling 3). Which one
/// once a second organisation shares this install is an open owner question; this is the one place to change.
pub async fn signup_organization(pool: &PgPool) -> Result<String, AppError> {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC' LIMIT 1")
        .fetch_optional(pool)
        .await
        .map_err(AppError::Database)?
        .ok_or_else(|| AppError::Internal("organisation XC not found".to_owned()))
}

/// Registers the Identity user behind `identifier` / `password` as a customer and signs them in: the session and
/// the new customer's id.
pub async fn register_customer(
    session: &SessionService,
    pool: &PgPool,
    input: SignupInput,
    client_ip: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(SignInTail, String), SignupError> {
    let (login, verified) = session
        .verified_login(&input.identifier, &input.password, client_ip)
        .await?;
    let org = signup_organization(pool).await.map_err(internal)?;
    let proven = verified_phone(&login);

    let mut tx = pool.begin().await.map_err(internal)?;
    let bound = CustomerBinder::bind_in(
        &mut tx,
        &org,
        &verified.subject,
        &proven,
        BindInput {
            display_name: input.display_name,
            lang: input.lang,
            marketing: input.marketing,
            evidence_ref: EVIDENCE.to_owned(),
            created_by: CREATED_BY.to_owned(),
        },
        now,
    )
    .await?;
    tx.commit().await.map_err(internal)?;

    let tail = session.sign_in_with_login(login, now).await?;
    Ok((tail, bound.customer_id))
}

fn verified_phone(login: &IdentityLoginResponse) -> Vec<ProvenContact> {
    let (Some(phone), Some(true)) = (login.phone.as_deref(), login.phone_verified) else {
        return Vec::new();
    };
    let Ok(e164) = normalize(IdentityType::Phone, phone, None) else {
        // Identity's phone is not one the desk can match; the register goes on without it.
        tracing::warn!("Identity's verified phone does not normalise; no PHONE identity written");
        return Vec::new();
    };
    vec![ProvenContact::Phone(e164)]
}
