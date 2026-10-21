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
use xc_customer::domain::consent::Channel;

/// A contact Identity (or the OTP challenge seal) proved this person holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenContact {
    /// E.164 (`normalize(IdentityType::Phone, …)`).
    Phone(String),
    /// Normalised address (`normalize(IdentityType::Email, …)`).
    Email(String),
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
        let _ = (tx, org, subject, proven, input, now);
        todo!("KWQ-C2-4: bind the user to a new or a joined customer")
    }
}
