//! `CustomerBinder` (design `kiosk-shared-phone.md` §4.2 D11, §4.3; owner Q1): binds a newly signed-up Identity user
//! to a customer of the organisation, in the caller's transaction. Shared by the app register
//! (`customer_signup::register_customer`) and the customer OTP door (`customer_otp_sign_in`), so the rules live once
//! (♻️):
//!
//! 1. The Identity subject must have no `users` row yet ⇒ else `AlreadyRegistered` (a concurrent bind lands here
//!    through `users.auth_subject UNIQUE`).
//! 2. One `users` row (ACTIVE, `auth_subject`, `created_by` = `input.created_by`), one organisation-wide CUSTOMER
//!    role and one `customerRegister` user-audit row, exactly as `register_customer` writes them today.
//! 3. Each proven contact is looked up by `LookupHasher(type, value)` among the organisation's ACTIVE
//!    `customer_identities`:
//!    - on a customer **with** a user ⇒ `PhoneInUse` / `EmailInUse`, nothing written;
//!    - on a desk-made customer with `user_id IS NULL` ⇒ that customer is **joined** (owner Q1): `customers.user_id`
//!      is set, TERMS (and the chosen MARKETING channels) are recorded on it through `ConsentService::record_in`
//!      with `input.evidence_ref`, and no new customer is made. [proposed] Its `display_name` and
//!      `preferred_language` are left as the desk wrote them.
//!    - [proposed] two proven contacts on two different desk customers ⇒ the PHONE's customer is joined and the
//!      EMAIL counts as in use (`EmailInUse`); see the open question on the ticket.
//! 4. Otherwise a new customer through `ConsentService::register_in` (TERMS, MARKETING), as today.
//! 5. Every proven contact not already ACTIVE on that customer is written as one ACTIVE identity: the value sealed
//!    with `PiiCipher` under AAD `customer_id ‖ TYPE`, `lookup_hash` = `LookupHasher(TYPE, value)`,
//!    `verified_at = now` — today's `write_verified_phone`, for PHONE and EMAIL alike. A unique violation on
//!    `customer_identities_one_active` ⇒ `PhoneInUse` / `EmailInUse`.
//!
//! No TERMS text (or MARKETING text, when channels are chosen) in `input.lang` ⇒ `ConsentTextMissing`. Any refusal
//! leaves the caller's transaction for the caller to drop: nothing is committed here. No identity value, hash or
//! secret is logged or put in an error.

use chrono::{DateTime, Utc};
use pos_error::AppError;
use xc_auth::application::session::session_service::generate_uuid;
use xc_auth::domain::access::{Role, ScopedRole};
use xc_auth::domain::user::{User, UserRepository, UserStatus};
use xc_auth::infrastructure::user::PgUserRepository;
use xc_customer::application::consent_service::{ConsentChoice, ConsentService, Registration};
use xc_customer::domain::consent::{Channel, Purpose};
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher};

use crate::application::customer_signup::is_consent_text_missing;

/// A contact Identity (or the OTP challenge seal) proved this person holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenContact {
    /// E.164 (`normalize(IdentityType::Phone, …)`).
    Phone(String),
    /// Normalised address (`normalize(IdentityType::Email, …)`).
    Email(String),
}

impl ProvenContact {
    fn parts(&self) -> (IdentityType, &str) {
        match self {
            Self::Phone(v) => (IdentityType::Phone, v.as_str()),
            Self::Email(v) => (IdentityType::Email, v.as_str()),
        }
    }
}

/// What the person typed when signing up.
#[derive(Debug, Clone)]
pub struct BindInput {
    pub display_name: String,
    pub lang: String,
    pub marketing: Vec<Channel>,
    /// `consents.evidence_ref`: `app:register` for the app register, `web:otp` for the OTP door.
    pub evidence_ref: String,
    /// `users.created_by`: `customer-register` for both doors today.
    pub created_by: String,
}

/// The bind's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    pub user_id: String,
    pub customer_id: String,
    /// A desk-made customer was joined (owner Q1) rather than a new one made.
    pub joined: bool,
}

/// Why a bind wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindError {
    AlreadyRegistered,
    ConsentTextMissing,
    PhoneInUse,
    EmailInUse,
    Internal(String),
}

fn unique_violation(e: &AppError) -> bool {
    matches!(e, AppError::Database(sqlx::Error::Database(d)) if d.code().as_deref() == Some("23505"))
}

fn in_use(t: IdentityType) -> BindError {
    match t {
        IdentityType::Phone => BindError::PhoneInUse,
        _ => BindError::EmailInUse,
    }
}

fn map_consent_err(e: AppError) -> BindError {
    if is_consent_text_missing(&e) {
        BindError::ConsentTextMissing
    } else {
        internal(e)
    }
}

fn internal(e: impl std::fmt::Display) -> BindError {
    BindError::Internal(e.to_string())
}

pub struct CustomerBinder;

impl CustomerBinder {
    /// Steps 1–5 of the module doc, in `tx`, for the organisation `org` and the Identity `subject`.
    pub async fn bind_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: &str,
        subject: &str,
        proven: &[ProvenContact],
        input: BindInput,
        now: DateTime<Utc>,
    ) -> Result<Bound, BindError> {
        let users = PgUserRepository::new();
        if users
            .find_by_auth_subject(tx, subject)
            .await
            .map_err(internal)?
            .is_some()
        {
            return Err(BindError::AlreadyRegistered);
        }

        let role = ScopedRole::new(Role::Customer, None, now, None);
        let user = User {
            id: generate_uuid(),
            organization_id: org.to_owned(),
            auth_subject: Some(subject.to_owned()),
            name: input.display_name.clone(),
            status: UserStatus::Active,
            version: 1,
            created_at: now,
            created_by: input.created_by,
            roles: vec![role.clone()],
        };
        match users.create_user(tx, &user).await {
            Ok(()) => {}
            Err(e) if unique_violation(&e) => return Err(BindError::AlreadyRegistered),
            Err(e) => return Err(internal(e)),
        }
        users
            .add_role(tx, &user.id, org, &role)
            .await
            .map_err(internal)?;
        users
            .append_audit(tx, &user.id, &user.id, "customerRegister", org)
            .await
            .map_err(internal)?;

        let keys = if proven.is_empty() {
            None
        } else {
            Some(xc_customer::get_pii_keys().map_err(internal)?)
        };

        // Order PHONE before EMAIL so two different desk customers join PHONE and refuse EMAIL.
        let mut ordered: Vec<&ProvenContact> = proven.iter().collect();
        ordered.sort_by_key(|c| matches!(c, ProvenContact::Email(_)));

        let mut joined_customer: Option<(String, IdentityType)> = None;
        let mut pending_identities: Vec<(IdentityType, &str, [u8; 32])> = Vec::new();

        if let Some(ref keys) = keys {
            let hasher = LookupHasher::new(keys);
            for contact in ordered {
                let (id_type, value) = contact.parts();
                let hash = hasher.hash(id_type, value);
                let existing: Option<(String, bool)> = sqlx::query_as(
                    "SELECT c.id, c.user_id IS NOT NULL \
                     FROM customer_identities ci \
                     JOIN customers c ON c.id = ci.customer_id \
                     WHERE ci.organization_id = $1::uuid \
                       AND ci.type = $2 \
                       AND ci.lookup_hash = $3 \
                       AND ci.status = 'ACTIVE'",
                )
                .bind(org)
                .bind(id_type.as_str())
                .bind(hash.as_slice())
                .fetch_optional(&mut **tx)
                .await
                .map_err(internal)?;

                match existing {
                    Some((_, true)) => return Err(in_use(id_type)),
                    Some((cid, false)) => match &joined_customer {
                        None => joined_customer = Some((cid, id_type)),
                        Some((joined, _)) if joined == &cid => {}
                        Some(_) => return Err(in_use(id_type)),
                    },
                    None => pending_identities.push((id_type, value, hash)),
                }
            }
        }

        let (customer_id, joined) = if let Some((customer_id, joined_by)) = joined_customer {
            let claimed = sqlx::query(
                "UPDATE customers SET user_id = $1::uuid \
                 WHERE id = $2 AND organization_id = $3::uuid AND user_id IS NULL",
            )
            .bind(&user.id)
            .bind(&customer_id)
            .bind(org)
            .execute(&mut **tx)
            .await
            .map_err(internal)?;
            // Another sign-in joined this desk customer after our read: it is in use now.
            if claimed.rows_affected() != 1 {
                return Err(in_use(joined_by));
            }

            ConsentService::record_in(
                tx,
                &customer_id,
                ConsentChoice {
                    purpose: Purpose::Terms,
                    channel: None,
                    granted: true,
                    lang: input.lang.clone(),
                    evidence_ref: input.evidence_ref.clone(),
                    recorded_by: None,
                },
                now,
            )
            .await
            .map_err(map_consent_err)?;

            for ch in input.marketing {
                ConsentService::record_in(
                    tx,
                    &customer_id,
                    ConsentChoice {
                        purpose: Purpose::Marketing,
                        channel: Some(ch),
                        granted: true,
                        lang: input.lang.clone(),
                        evidence_ref: input.evidence_ref.clone(),
                        recorded_by: None,
                    },
                    now,
                )
                .await
                .map_err(map_consent_err)?;
            }

            (customer_id, true)
        } else {
            let customer_id = ConsentService::register_in(
                tx,
                org,
                Registration {
                    user_id: user.id.clone(),
                    display_name: input.display_name,
                    lang: input.lang,
                    marketing: input.marketing,
                    evidence_ref: input.evidence_ref,
                },
                now,
            )
            .await
            .map_err(map_consent_err)?;
            (customer_id, false)
        };

        if let Some(keys) = keys {
            let cipher = PiiCipher::new(keys);
            for (id_type, value, hash) in pending_identities {
                let sealed = cipher.encrypt(
                    format!("{customer_id}{}", id_type.as_str()).as_bytes(),
                    value.as_bytes(),
                );
                let inserted = sqlx::query(
                    "INSERT INTO customer_identities (id, organization_id, customer_id, type, \
                         encrypted_value, key_id, lookup_hash, verified_at, status) \
                     VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, $5, $6, $7, 'ACTIVE')",
                )
                .bind(org)
                .bind(&customer_id)
                .bind(id_type.as_str())
                .bind(&sealed.bytes)
                .bind(&sealed.key_id)
                .bind(hash.as_slice())
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database);

                match inserted {
                    Ok(_) => {}
                    Err(e) if unique_violation(&e) => return Err(in_use(id_type)),
                    Err(e) => return Err(internal(e)),
                }
            }
        }

        Ok(Bound {
            user_id: user.id,
            customer_id,
            joined,
        })
    }
}
