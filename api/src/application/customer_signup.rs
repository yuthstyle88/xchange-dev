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
use xc_auth::application::session::session_service::generate_uuid;
use xc_auth::application::session::{SessionError, SessionService, SignInTail};
use xc_auth::domain::access::{Role, ScopedRole};
use xc_auth::domain::identity::identity_client::IdentityLoginResponse;
use xc_auth::domain::user::{User, UserRepository, UserStatus};
use xc_auth::infrastructure::user::PgUserRepository;
use xc_customer::application::consent_service::{ConsentService, Registration};
use xc_customer::domain::consent::{Channel, ConsentError};
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, normalize};

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

/// Who wrote a customer's own user and role, in `users.created_by` and `user_audit`.
const CREATED_BY: &str = "customer-register";
/// The consent evidence of a sign-up made in the app.
const EVIDENCE: &str = "app:register";

fn unique_violation(e: &AppError) -> bool {
    matches!(e, AppError::Database(sqlx::Error::Database(d)) if d.code().as_deref() == Some("23505"))
}

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
    let users = PgUserRepository::new();

    let mut tx = pool.begin().await.map_err(internal)?;
    if users
        .find_by_auth_subject(&mut tx, &verified.subject)
        .await
        .map_err(internal)?
        .is_some()
    {
        return Err(SignupError::AlreadyRegistered);
    }
    let role = ScopedRole::new(Role::Customer, None, now, None);
    let user = User {
        id: generate_uuid(),
        organization_id: org.clone(),
        auth_subject: Some(verified.subject.clone()),
        name: input.display_name.clone(),
        status: UserStatus::Active,
        version: 1,
        created_at: now,
        created_by: CREATED_BY.to_owned(),
        legacy_employee_id: None,
        roles: vec![role.clone()],
    };
    // A concurrent register of the same subject waits on `users.auth_subject UNIQUE` and lands here (R13).
    match users.create_user(&mut tx, &user).await {
        Ok(()) => {}
        Err(e) if unique_violation(&e) => return Err(SignupError::AlreadyRegistered),
        Err(e) => return Err(internal(e)),
    }
    users
        .add_role(&mut tx, &user.id, &org, &role)
        .await
        .map_err(internal)?;
    users
        .append_audit(&mut tx, &user.id, &user.id, "customerRegister", &org)
        .await
        .map_err(internal)?;

    let customer_id = ConsentService::register_in(
        &mut tx,
        &org,
        Registration {
            user_id: user.id.clone(),
            display_name: input.display_name,
            lang: input.lang,
            marketing: input.marketing,
            evidence_ref: EVIDENCE.to_owned(),
        },
        now,
    )
    .await
    .map_err(|e| {
        if is_consent_text_missing(&e) {
            SignupError::ConsentTextMissing
        } else {
            internal(e)
        }
    })?;

    write_verified_phone(&mut tx, &org, &customer_id, &login, now).await?;
    tx.commit().await.map_err(internal)?;

    let tail = session.sign_in_with_login(login, now).await?;
    Ok((tail, customer_id))
}

/// One ACTIVE PHONE identity when Identity reports the phone as verified (07:42Z): the E.164 text sealed with AAD
/// `customer_id ‖ "PHONE"`, `lookup_hash = LookupHasher(PHONE, e164)`, `verified_at = now`. Nothing otherwise.
async fn write_verified_phone(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: &str,
    customer_id: &str,
    login: &IdentityLoginResponse,
    now: DateTime<Utc>,
) -> Result<(), SignupError> {
    let (Some(phone), Some(true)) = (login.phone.as_deref(), login.phone_verified) else {
        return Ok(());
    };
    let Ok(e164) = normalize(IdentityType::Phone, phone, None) else {
        // Identity's phone is not one the desk can match; the register goes on without it.
        tracing::warn!("Identity's verified phone does not normalise; no PHONE identity written");
        return Ok(());
    };
    let keys = xc_customer::get_pii_keys().map_err(internal)?;
    let hash = LookupHasher::new(&keys).hash(IdentityType::Phone, &e164);

    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM customer_identities \
         WHERE organization_id = $1::uuid AND type = 'PHONE' AND lookup_hash = $2 AND status = 'ACTIVE')",
    )
    .bind(org)
    .bind(hash.as_slice())
    .fetch_one(&mut **tx)
    .await
    .map_err(internal)?;
    if taken {
        return Err(SignupError::PhoneInUse);
    }

    let sealed = PiiCipher::new(keys).encrypt(
        format!("{customer_id}{}", IdentityType::Phone.as_str()).as_bytes(),
        e164.as_bytes(),
    );
    let inserted = sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status) \
         VALUES (gen_random_uuid(), $1::uuid, $2, 'PHONE', $3, $4, $5, $6, 'ACTIVE')",
    )
    .bind(org)
    .bind(customer_id)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(hash.as_slice())
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database);
    match inserted {
        Ok(_) => Ok(()),
        // Another register bound the same phone between the check and the insert.
        Err(e) if unique_violation(&e) => Err(SignupError::PhoneInUse),
        Err(e) => Err(internal(e)),
    }
}
