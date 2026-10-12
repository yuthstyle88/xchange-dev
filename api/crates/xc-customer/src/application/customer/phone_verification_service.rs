//! Phone verification application service (CU-P1, design §6.3).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rand::Rng;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher};

use crate::domain::phone_otp_policy::PhoneOtpPolicy;
use crate::domain::phone_verification::PhoneVerification;
use crate::domain::sms_sender::SmsSender;
use crate::infrastructure::customer::phone_verification_repository::{
    InsertChallengeParams, InsertIdentityParams, PgPhoneVerificationRepository,
};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PhoneVerificationError {
    #[error("validationFailed")]
    ValidationFailed(String),
    #[error("codeExpired")]
    CodeExpired,
    #[error("tooManyAttempts")]
    TooManyAttempts,
    #[error("rateLimited")]
    RateLimited,
    #[error("phoneInUse")]
    PhoneInUse,
    #[error("smsUnavailable")]
    SmsUnavailable,
    #[error("outOfScope")]
    OutOfScope,
    #[error("notFound")]
    NotFound,
    #[error("roleNotAllowed")]
    RoleNotAllowed,
    #[error("wrongCode")]
    WrongCode,
    #[error("superseded")]
    Superseded,
    #[error("internal: {0}")]
    Internal(String),
    #[error("database error")]
    Database,
}

impl PhoneVerificationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ValidationFailed(_) => "validationFailed",
            Self::CodeExpired => "codeExpired",
            Self::TooManyAttempts => "tooManyAttempts",
            Self::RateLimited => "rateLimited",
            Self::PhoneInUse => "phoneInUse",
            Self::SmsUnavailable => "smsUnavailable",
            Self::OutOfScope => "outOfScope",
            Self::NotFound => "notFound",
            Self::RoleNotAllowed => "roleNotAllowed",
            Self::WrongCode => "wrongCode",
            Self::Superseded => "superseded",
            Self::Internal(_) => "internal",
            Self::Database => "database",
        }
    }

    pub fn status_code(&self) -> u16 {
        match self {
            Self::ValidationFailed(_) | Self::RoleNotAllowed => 400,
            Self::OutOfScope => 403,
            Self::NotFound => 404,
            Self::CodeExpired
            | Self::TooManyAttempts
            | Self::WrongCode
            | Self::Superseded
            | Self::PhoneInUse => 409,
            Self::RateLimited => 429,
            Self::SmsUnavailable => 503,
            Self::Internal(_) | Self::Database => 500,
        }
    }
}

impl From<sqlx::Error> for PhoneVerificationError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}

impl From<pos_error::AppError> for PhoneVerificationError {
    fn from(e: pos_error::AppError) -> Self {
        Self::Internal(e.to_string())
    }
}

impl From<PhoneVerificationError> for pos_error::AppError {
    fn from(e: PhoneVerificationError) -> Self {
        match e {
            PhoneVerificationError::Internal(msg) => pos_error::AppError::Internal(msg),
            other => pos_error::AppError::Domain(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhoneStartResult {
    pub challenge_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhoneConfirmResult {
    pub verified: bool,
    pub identity_id: Uuid,
}

pub struct PhoneVerificationService {
    branch_tx: BranchTx,
    sms_sender: Arc<dyn SmsSender>,
    repo: PgPhoneVerificationRepository,
}

fn actor_role(actor: &Actor) -> String {
    actor
        .roles
        .first()
        .map(|r| r.role.as_str().to_string())
        .unwrap_or_default()
}

impl PhoneVerificationService {
    pub fn new(branch_tx: BranchTx, sms_sender: Arc<dyn SmsSender>) -> Self {
        let repo = PgPhoneVerificationRepository::new(branch_tx.pool().clone());
        Self {
            branch_tx,
            sms_sender,
            repo,
        }
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub fn sms_sender(&self) -> &Arc<dyn SmsSender> {
        &self.sms_sender
    }

    pub fn repo(&self) -> &PgPhoneVerificationRepository {
        &self.repo
    }

    pub async fn start(
        &self,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        phone: &str,
        now: DateTime<Utc>,
    ) -> Result<PhoneStartResult, PhoneVerificationError> {
        let mut tx = self.repo.pool().begin().await?;
        let res = self
            .start_in_tx(&mut tx, actor, branch, customer_id, phone, now)
            .await?;
        tx.commit().await?;
        Ok(res)
    }

    pub async fn start_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        phone: &str,
        now: DateTime<Utc>,
    ) -> Result<PhoneStartResult, PhoneVerificationError> {
        let org_id = Uuid::parse_str(&actor.organization_id)
            .map_err(|_| PhoneVerificationError::NotFound)?;
        let teller_user_id = Uuid::parse_str(&actor.user_id)
            .map_err(|_| PhoneVerificationError::Internal("invalid teller user id".into()))?;

        // 1. Lock customer row for update in the transaction
        let preferred_language = match self.repo.lock_customer(tx, customer_id, org_id).await? {
            Some(lang) => lang,
            None => return Err(PhoneVerificationError::NotFound),
        };

        // 2. Normalize phone and validate length (E.164 8..=15 digits)
        let normalized = match xc_kernel::pii::normalize(IdentityType::Phone, phone, None) {
            Ok(p) => {
                let digits = p.strip_prefix('+').unwrap_or(&p);
                if digits.len() >= 8
                    && digits.len() <= 15
                    && digits.chars().all(|c| c.is_ascii_digit())
                {
                    p
                } else {
                    let _ = self
                        .write_audit_event_standalone(
                            &teller_user_id,
                            actor,
                            branch,
                            customer_id,
                            None,
                            "refused",
                        )
                        .await;
                    return Err(PhoneVerificationError::ValidationFailed(
                        "invalid phone number".into(),
                    ));
                }
            }
            Err(_) => {
                let _ = self
                    .write_audit_event_standalone(
                        &teller_user_id,
                        actor,
                        branch,
                        customer_id,
                        None,
                        "refused",
                    )
                    .await;
                return Err(PhoneVerificationError::ValidationFailed(
                    "invalid phone number".into(),
                ));
            }
        };

        // 3. Check rate limits inside the transaction
        let cust_window =
            now - chrono::Duration::minutes(PhoneOtpPolicy::MAX_PER_CUSTOMER_WINDOW_MINUTES);
        let cust_count = self
            .repo
            .count_customer_challenges(tx, org_id, customer_id, cust_window)
            .await?;
        if cust_count >= PhoneOtpPolicy::MAX_PER_CUSTOMER_CHALLENGES {
            let _ = self
                .write_audit_event_standalone(
                    &teller_user_id,
                    actor,
                    branch,
                    customer_id,
                    None,
                    "refused",
                )
                .await;
            return Err(PhoneVerificationError::RateLimited);
        }

        let keys = crate::pii_keys::get_pii_keys()?;
        let phone_lookup_hash = LookupHasher::new(&keys).hash(IdentityType::Phone, &normalized);
        let phone_window =
            now - chrono::Duration::hours(PhoneOtpPolicy::MAX_PER_PHONE_WINDOW_HOURS);
        let phone_count = self
            .repo
            .count_phone_challenges(tx, org_id, &phone_lookup_hash, phone_window)
            .await?;
        if phone_count >= PhoneOtpPolicy::MAX_PER_PHONE_CHALLENGES {
            let _ = self
                .write_audit_event_standalone(
                    &teller_user_id,
                    actor,
                    branch,
                    customer_id,
                    None,
                    "refused",
                )
                .await;
            return Err(PhoneVerificationError::RateLimited);
        }

        let teller_window =
            now - chrono::Duration::hours(PhoneOtpPolicy::MAX_PER_TELLER_WINDOW_HOURS);
        let teller_count = self
            .repo
            .count_teller_challenges(tx, org_id, teller_user_id, teller_window)
            .await?;
        if teller_count >= PhoneOtpPolicy::MAX_PER_TELLER_CHALLENGES {
            let _ = self
                .write_audit_event_standalone(
                    &teller_user_id,
                    actor,
                    branch,
                    customer_id,
                    None,
                    "refused",
                )
                .await;
            return Err(PhoneVerificationError::RateLimited);
        }

        // 4. Branch name for SMS
        let branch_name = self
            .repo
            .fetch_branch_name(tx, branch.as_str())
            .await?
            .unwrap_or_else(|| branch.as_str().to_string());

        // 5. Generate 6-digit OTP code & challenge id using rand
        let challenge_id = Uuid::new_v4();
        let code_num = rand::thread_rng().gen_range(0..1_000_000);
        let code = format!("{:06}", code_num);

        let code_hash = PhoneVerification::try_compute_code_hash(challenge_id, &code)?;
        let cipher = PiiCipher::new(keys.clone());
        let aad = format!("{customer_id}PHONE_OTP");
        let sealed = cipher.encrypt(aad.as_bytes(), normalized.as_bytes());

        // 6. Send SMS before writing challenge row (Notify failure aborts without challenge row)
        let sms_text = PhoneOtpPolicy::sms_text(&preferred_language, &code, &branch_name);
        if let Err(_e) = self.sms_sender.send(&normalized, &sms_text).await {
            let _ = self
                .write_audit_event_standalone(
                    &teller_user_id,
                    actor,
                    branch,
                    customer_id,
                    None,
                    "refused",
                )
                .await;
            return Err(PhoneVerificationError::SmsUnavailable);
        }

        // 7. Persist challenge & audit log in transaction
        let expires_at = now + chrono::Duration::minutes(PhoneOtpPolicy::EXPIRY_MINUTES);

        self.repo
            .supersede_pending(tx, org_id, customer_id, now)
            .await?;

        self.repo
            .insert_challenge(
                tx,
                InsertChallengeParams {
                    id: challenge_id,
                    organization_id: org_id,
                    customer_id,
                    branch_id: branch.as_str(),
                    phone_encrypted: &sealed.bytes,
                    key_id: &sealed.key_id,
                    phone_lookup_hash: &phone_lookup_hash,
                    code_hash: &code_hash,
                    expires_at,
                    teller_user_id,
                    created_at: now,
                },
            )
            .await?;

        self.write_audit_event_in_tx(
            tx,
            &teller_user_id,
            actor,
            branch,
            customer_id,
            Some(challenge_id),
            "sent",
        )
        .await?;

        Ok(PhoneStartResult {
            challenge_id,
            expires_at,
        })
    }

    pub async fn confirm(
        &self,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        challenge_id: Uuid,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<PhoneConfirmResult, PhoneVerificationError> {
        let mut tx = self.repo.pool().begin().await?;
        let res = self
            .confirm_in_tx(&mut tx, actor, branch, customer_id, challenge_id, code, now)
            .await?;
        tx.commit().await?;
        Ok(res)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn confirm_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        challenge_id: Uuid,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<PhoneConfirmResult, PhoneVerificationError> {
        let org_id = Uuid::parse_str(&actor.organization_id)
            .map_err(|_| PhoneVerificationError::NotFound)?;
        let teller_user_id = Uuid::parse_str(&actor.user_id)
            .map_err(|_| PhoneVerificationError::Internal("invalid teller user id".into()))?;

        // 1. Fetch challenge row (snapshot read without FOR UPDATE so standalone tx doesn't deadlock)
        let challenge = match self.repo.fetch_challenge(tx, challenge_id).await? {
            Some(row) => row,
            None => return Err(PhoneVerificationError::NotFound),
        };

        if challenge.organization_id != org_id || challenge.customer_id != customer_id {
            return Err(PhoneVerificationError::NotFound);
        }

        // 2. Check status
        if challenge.status == "LOCKED" {
            return Err(PhoneVerificationError::TooManyAttempts);
        }
        if challenge.status == "SUPERSEDED" {
            return Err(PhoneVerificationError::Superseded);
        }
        if challenge.status != "PENDING" {
            return Err(PhoneVerificationError::ValidationFailed(
                "challenge is not pending".into(),
            ));
        }

        // 3. Check expiry
        if now > challenge.expires_at {
            return Err(PhoneVerificationError::CodeExpired);
        }

        // 4. Constant-time code hash compare
        let expected_hash = PhoneVerification::try_compute_code_hash(challenge_id, code)?;
        let matches = constant_time_eq(&challenge.code_hash, &expected_hash);

        if !matches {
            let err = self
                .record_wrong_code_standalone(
                    &teller_user_id,
                    actor,
                    branch,
                    customer_id,
                    challenge_id,
                    now,
                )
                .await?;
            return Err(err);
        }

        // 5. Code matched! Lock challenge row with FOR UPDATE in tx
        let challenge = match self.repo.lock_challenge(tx, challenge_id).await? {
            Some(row) => row,
            None => return Err(PhoneVerificationError::NotFound),
        };

        if challenge.status == "LOCKED" {
            return Err(PhoneVerificationError::TooManyAttempts);
        }
        if challenge.status == "SUPERSEDED" {
            return Err(PhoneVerificationError::Superseded);
        }
        if challenge.status != "PENDING" {
            return Err(PhoneVerificationError::ValidationFailed(
                "challenge is not pending".into(),
            ));
        }

        // 6. Check phoneInUse by another customer in same org
        if let Some(existing_cust) = self
            .repo
            .find_active_phone_identity(tx, org_id, &challenge.phone_lookup_hash)
            .await?
        {
            if existing_cust != customer_id {
                return Err(PhoneVerificationError::PhoneInUse);
            }
        }

        // 6. In same transaction: revoke old active phone, activate new phone identity, mark VERIFIED
        self.repo
            .revoke_active_phone_identities(tx, org_id, customer_id, now)
            .await?;

        let keys = crate::pii_keys::get_pii_keys()?;
        let cipher = PiiCipher::new(keys.clone());
        let challenge_aad = format!("{customer_id}PHONE_OTP");
        let sealed_challenge = xc_kernel::pii::Sealed {
            key_id: challenge.key_id.clone(),
            bytes: challenge.phone_encrypted.clone(),
        };
        let plain_phone_bytes = cipher
            .decrypt(challenge_aad.as_bytes(), &sealed_challenge)
            .map_err(|e| {
                PhoneVerificationError::Internal(format!("failed to decrypt challenge phone: {e}"))
            })?;

        let identity_aad = format!("{customer_id}PHONE");
        let sealed_identity = cipher.encrypt(identity_aad.as_bytes(), &plain_phone_bytes);

        let identity_id = Uuid::new_v4();
        self.repo
            .insert_customer_identity(
                tx,
                InsertIdentityParams {
                    id: identity_id,
                    organization_id: org_id,
                    customer_id,
                    encrypted_value: &sealed_identity.bytes,
                    key_id: &sealed_identity.key_id,
                    lookup_hash: &challenge.phone_lookup_hash,
                    now,
                },
            )
            .await?;

        let rows_affected = self
            .repo
            .verify_challenge(tx, challenge_id, identity_id, now)
            .await?;
        if rows_affected == 0 {
            return Err(PhoneVerificationError::ValidationFailed(
                "challenge is not pending".into(),
            ));
        }

        // Append audit event "verified"
        self.write_audit_event_in_tx(
            tx,
            &teller_user_id,
            actor,
            branch,
            customer_id,
            Some(challenge_id),
            "verified",
        )
        .await?;

        Ok(PhoneConfirmResult {
            verified: true,
            identity_id,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_audit_event_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        teller_user_id: &Uuid,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        challenge_id: Option<Uuid>,
        outcome: &str,
    ) -> Result<(), PhoneVerificationError> {
        let audit_log = PgAuditLog::new(self.repo.pool().clone());
        let actor_role = actor_role(actor);

        let after = if let Some(cid) = challenge_id {
            json!({
                "outcome": outcome,
                "customer_id": customer_id,
                "challenge_id": cid.to_string(),
            })
        } else {
            json!({
                "outcome": outcome,
                "customer_id": customer_id,
            })
        };

        let event = AuditEvent {
            actor_id: teller_user_id.to_string(),
            actor_role,
            scope: "BRANCH".to_string(),
            branch_id: Some(branch.as_str().to_string()),
            action: "customer.phone_verification".to_string(),
            object_ref: challenge_id
                .map(|c| c.to_string())
                .unwrap_or_else(|| customer_id.to_string()),
            before: None,
            after: Some(after),
        };

        audit_log
            .append(tx, &event)
            .await
            .map_err(|e| PhoneVerificationError::Internal(e.to_string()))?;
        Ok(())
    }

    async fn write_audit_event_standalone(
        &self,
        teller_user_id: &Uuid,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        challenge_id: Option<Uuid>,
        outcome: &str,
    ) -> Result<(), PhoneVerificationError> {
        let mut tx = self.repo.pool().begin().await?;
        self.write_audit_event_in_tx(
            &mut tx,
            teller_user_id,
            actor,
            branch,
            customer_id,
            challenge_id,
            outcome,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_wrong_code_standalone(
        &self,
        teller_user_id: &Uuid,
        actor: &Actor,
        branch: &BranchId,
        customer_id: &str,
        challenge_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<PhoneVerificationError, PhoneVerificationError> {
        let mut tx = self.repo.pool().begin().await?;

        // 1. Lock challenge row with FOR UPDATE in this standalone transaction
        let challenge = match self.repo.lock_challenge(&mut tx, challenge_id).await? {
            Some(row) => row,
            None => {
                let _ = tx.rollback().await;
                return Ok(PhoneVerificationError::NotFound);
            }
        };

        if challenge.status == "LOCKED" {
            let _ = tx.rollback().await;
            return Ok(PhoneVerificationError::TooManyAttempts);
        }

        let updated = self
            .repo
            .atomic_record_wrong_code(&mut tx, challenge_id, PhoneOtpPolicy::MAX_ATTEMPTS, now)
            .await?;

        let (_new_attempts, new_status) = match updated {
            Some(pair) => pair,
            None => {
                let _ = tx.rollback().await;
                return Ok(PhoneVerificationError::TooManyAttempts);
            }
        };

        let outcome = if new_status == "LOCKED" {
            "locked"
        } else {
            "wrongCode"
        };

        self.write_audit_event_in_tx(
            &mut tx,
            teller_user_id,
            actor,
            branch,
            customer_id,
            Some(challenge_id),
            outcome,
        )
        .await?;

        tx.commit().await?;

        if new_status == "LOCKED" {
            Ok(PhoneVerificationError::TooManyAttempts)
        } else {
            Ok(PhoneVerificationError::WrongCode)
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
