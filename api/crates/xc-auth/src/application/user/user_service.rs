//! User application service.

use std::sync::Arc;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::access::{Role, ScopeRule, ScopedRole};
use crate::domain::user::{Invite, User, UserRepository, UserStatus};
use crate::infrastructure::user::PgUserRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InviteRefusal {
    InviteInvalid,
    AlreadyLinked,
    UserNotFound,
    UserDisabled,
}

impl InviteRefusal {
    pub fn message(&self) -> &'static str {
        match self {
            Self::InviteInvalid => "รหัสเชิญไม่ถูกต้อง ถูกใช้งานแล้ว หรือหมดอายุแล้ว",
            Self::AlreadyLinked => "ผู้ใช้นี้ลงทะเบียนแล้ว",
            Self::UserNotFound => "ไม่พบผู้ใช้งาน",
            Self::UserDisabled => "บัญชีถูกระงับ",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    #[error("invite refused: {0:?}")]
    Refused(InviteRefusal),
    #[error(transparent)]
    App(#[from] AppError),
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_uuid() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // v4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Clone)]
pub struct UserService<R = PgUserRepository> {
    repo: Arc<R>,
}

impl<R> UserService<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &R {
        &self.repo
    }
}

impl<R: UserRepository> UserService<R> {
    /// Invites a new user: validates initial roles with ScopeRule, inserts user (status INVITED),
    /// generates a one-time invite code and stores its hash, and assigns the roles.
    /// Returns the created User and the plaintext invite code.
    #[allow(clippy::too_many_arguments)]
    pub async fn invite(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: &str,
        name: &str,
        roles: Vec<ScopedRole>,
        created_by: &str,
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> AppResult<(User, String)> {
        let mut accumulated: Vec<ScopedRole> = Vec::new();
        for r in &roles {
            ScopeRule::check(r.role, r.branch.as_ref(), &accumulated).map_err(|e| {
                AppError::Domain(format!("Scope rule rejected role {:?}: {e}", r.role))
            })?;
            accumulated.push(r.clone());
        }

        let user_id = generate_uuid();
        let invite_code = generate_token();
        let code_hash = sha256_hex(&invite_code);

        let user = User {
            id: user_id.clone(),
            organization_id: organization_id.to_string(),
            auth_subject: None,
            name: name.to_string(),
            status: UserStatus::Invited,
            version: 1,
            created_at: now,
            created_by: created_by.to_string(),
            legacy_employee_id: None,
            roles: roles.clone(),
        };

        self.repo.create_user(tx, &user).await?;

        let invite = Invite::new(&user_id, code_hash, expires_at);
        self.repo.create_invite(tx, &invite).await?;

        for r in &roles {
            self.repo.add_role(tx, &user_id, organization_id, r).await?;
        }

        Ok((user, invite_code))
    }

    /// Claims an invite code with an authenticated Identity subject:
    /// verifies the code is unused and unexpired, checks subject uniqueness,
    /// sets auth_subject, sets status = ACTIVE, bumps version, and marks code used.
    pub async fn claim(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        invite_code: &str,
        auth_subject: &str,
        now: DateTime<Utc>,
    ) -> Result<User, ClaimError> {
        let code_hash = sha256_hex(invite_code);

        // Spend the invite atomically as the FIRST statement in the claim transaction.
        // Takes row-level lock on user_invites. If already used, expired, or claimed concurrently,
        // returns None and is refused immediately.
        let user_id = self
            .repo
            .spend_invite(tx, &code_hash, now)
            .await?
            .ok_or(ClaimError::Refused(InviteRefusal::InviteInvalid))?;

        if self
            .repo
            .find_by_auth_subject(tx, auth_subject)
            .await?
            .is_some()
        {
            return Err(ClaimError::Refused(InviteRefusal::AlreadyLinked));
        }

        let mut user = self
            .repo
            .find_by_id(tx, &user_id)
            .await?
            .ok_or(ClaimError::Refused(InviteRefusal::UserNotFound))?;

        if user.status == UserStatus::Disabled {
            return Err(ClaimError::Refused(InviteRefusal::UserDisabled));
        }

        self.repo
            .set_auth_subject(tx, &user.id, auth_subject)
            .await?;
        self.repo
            .set_status(tx, &user.id, UserStatus::Active)
            .await?;

        user.auth_subject = Some(auth_subject.to_string());
        user.status = UserStatus::Active;
        user.version += 1;

        Ok(user)
    }

    /// Grants a new scoped role to a user:
    /// checks ScopeRule against current active roles before writing,
    /// writes user_roles row, and bumps users.version.
    pub async fn grant(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        role: Role,
        branch: Option<BranchId>,
        valid_from: DateTime<Utc>,
        valid_to: Option<DateTime<Utc>>,
    ) -> AppResult<ScopedRole> {
        let user = self
            .repo
            .find_by_id(tx, user_id)
            .await?
            .ok_or_else(|| AppError::Domain("ไม่พบผู้ใช้งาน".to_string()))?;

        if user.status == UserStatus::Disabled {
            return Err(AppError::Domain("บัญชีถูกระงับ".to_string()));
        }

        let existing_roles = self.repo.find_roles_by_user_id(tx, user_id).await?;
        let active_existing: Vec<ScopedRole> = existing_roles
            .into_iter()
            .filter(|r| r.valid_to.is_none_or(|to| valid_from < to))
            .collect();

        ScopeRule::check(role, branch.as_ref(), &active_existing)
            .map_err(|e| AppError::Domain(format!("Scope rule rejected: {e}")))?;

        let scoped_role = ScopedRole::new(role, branch, valid_from, valid_to);
        self.repo
            .add_role(tx, user_id, &user.organization_id, &scoped_role)
            .await?;
        self.repo.bump_version(tx, user_id).await?;

        Ok(scoped_role)
    }

    /// Revokes a role by setting valid_to on matching user_roles rows,
    /// and bumps users.version.
    pub async fn revoke(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
        role: Role,
        branch: Option<&BranchId>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let user = self
            .repo
            .find_by_id(tx, user_id)
            .await?
            .ok_or_else(|| AppError::Domain("ไม่พบผู้ใช้งาน".to_string()))?;

        self.repo
            .revoke_role(tx, &user.id, role, branch, now)
            .await?;
        self.repo.bump_version(tx, &user.id).await?;

        Ok(())
    }

    /// Disables a user: sets status = DISABLED and bumps users.version.
    pub async fn disable(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: &str,
    ) -> AppResult<()> {
        let user = self
            .repo
            .find_by_id(tx, user_id)
            .await?
            .ok_or_else(|| AppError::Domain("ไม่พบผู้ใช้งาน".to_string()))?;

        self.repo
            .set_status(tx, &user.id, UserStatus::Disabled)
            .await?;

        Ok(())
    }

    /// Looks up a user by ID.
    pub async fn get_user(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: &str,
    ) -> AppResult<Option<User>> {
        self.repo.find_by_id(tx, id).await
    }

    /// Looks up a user by auth_subject.
    pub async fn get_user_by_auth_subject(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        subject: &str,
    ) -> AppResult<Option<User>> {
        self.repo.find_by_auth_subject(tx, subject).await
    }
}
