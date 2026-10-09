use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::application::user::UserService;
use crate::application::user::user_service::{ClaimError, InviteRefusal};
use crate::domain::access::{Role, ScopedRole};
use crate::domain::identity::identity_client::{
    IdentityClient, IdentityError, IdentityLoginResponse, IdentityOtpRequestResponse,
    IdentityRegisterResponse, UNAVAILABLE_MESSAGE,
};
use crate::domain::token::TokenVerifier;
use crate::domain::user::{User, UserStatus};
use crate::infrastructure::user::PgUserRepository;

use super::refresh_token::{decrypt_refresh_token, encrypt_refresh_token};

pub const SESSION_SECONDS: i64 = 43_200;
pub const SESSION_COOKIE: &str = "__Host-xc_session";
pub const OWNER_COOKIE: &str = "__Host-xc_owner";
pub const STAFF_COOKIE: &str = "__Host-xc_staff";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUserRole {
    pub role: String,
    #[serde(rename = "branchId")]
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUser {
    pub id: String,
    pub name: String,
    #[serde(rename = "organizationId")]
    pub organization_id: String,
    pub roles: Vec<SessionUserRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInTail {
    pub user: SessionUser,
    pub session_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    Identity(IdentityError),
    UserDisabled,
    UserNotBound,
    InviteRefused(InviteRefusal),
    Database(String),
}

impl From<IdentityError> for SessionError {
    fn from(err: IdentityError) -> Self {
        Self::Identity(err)
    }
}

impl SessionError {
    pub fn refusal(&self) -> (u16, &'static str, String) {
        match self {
            Self::Identity(err) => {
                let (status, code, msg) = err.refusal();
                (status, code, msg.to_string())
            }
            Self::UserDisabled => (403, "userDisabled", "บัญชีถูกระงับ".to_string()),
            Self::UserNotBound => (
                401,
                "signIn",
                "บัญชีนี้ยังไม่ได้ผูกกับผู้ใช้ของ XCHANGE กรุณาใช้รหัสเชิญ".to_string(),
            ),
            Self::InviteRefused(reason) => (400, "inviteRefused", reason.message().to_string()),
            Self::Database(_) => (503, "identityUnavailable", UNAVAILABLE_MESSAGE.to_string()),
        }
    }

    pub fn reason(&self) -> Option<InviteRefusal> {
        match self {
            Self::InviteRefused(reason) => Some(*reason),
            _ => None,
        }
    }
}

pub fn generate_session_token() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes).expect("random bytes");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn generate_uuid() -> String {
    let mut bytes = [0u8; 16];
    let rng = SystemRandom::new();
    rng.fill(&mut bytes).expect("random bytes");
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

type SessionCheckRow = (String, String, String, String, i32, i32, DateTime<Utc>);
type RefreshSessionRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    i32,
    i32,
    DateTime<Utc>,
    Option<Vec<u8>>,
);
type ActiveUserRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    i32,
    i32,
    DateTime<Utc>,
);
type RoleRow = (String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>);

#[derive(Clone)]
pub struct SessionService {
    pool: PgPool,
    identity: Option<Arc<dyn IdentityClient>>,
    verifier: Option<Arc<dyn TokenVerifier>>,
    client_secret: Option<String>,
}

#[allow(clippy::type_complexity)]
impl SessionService {
    pub fn new(
        pool: PgPool,
        identity: Option<Arc<dyn IdentityClient>>,
        verifier: Option<Arc<dyn TokenVerifier>>,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            pool,
            identity,
            verifier,
            client_secret,
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn identity_client(&self) -> Option<&Arc<dyn IdentityClient>> {
        self.identity.as_ref()
    }

    pub fn is_configured(&self) -> bool {
        self.identity.is_some()
    }

    async fn fetch_user_roles(
        pool: &PgPool,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<SessionUserRole>, sqlx::Error> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT role_code, branch_id FROM user_roles \
             WHERE user_id = $1::uuid AND valid_from <= $2 AND (valid_to IS NULL OR valid_to > $2) \
             ORDER BY valid_from ASC, id ASC",
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(role, branch_id)| SessionUserRole { role, branch_id })
            .collect())
    }

    async fn create_session_for_user(
        &self,
        user_id: &str,
        organization_id: &str,
        name: &str,
        version: i32,
        refresh_token: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        let token = generate_session_token();
        let token_hash = sha256_hex(&token);
        let expires = now + Duration::seconds(SESSION_SECONDS);

        let refresh_enc = match (self.client_secret.as_deref(), refresh_token) {
            (Some(secret), Some(rt)) => {
                let enc = encrypt_refresh_token(secret, rt).map_err(|_| {
                    SessionError::Identity(IdentityError::unavailable(
                        "/auth/login",
                        503,
                        "encrypt_failed",
                    ))
                })?;
                Some(enc)
            }
            _ => None,
        };

        sqlx::query(
            "INSERT INTO user_sessions (hash, user_id, version, expires, refresh_token_enc, organization_id) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(version)
        .bind(expires)
        .bind(refresh_enc)
        .bind(organization_id)
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let roles = Self::fetch_user_roles(&self.pool, user_id, now)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        Ok(SignInTail {
            user: SessionUser {
                id: user_id.to_string(),
                name: name.to_string(),
                organization_id: organization_id.to_string(),
                roles,
            },
            session_token: token,
        })
    }

    pub async fn sign_in_with_login(
        &self,
        login: IdentityLoginResponse,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        let verifier = self.verifier.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/auth/login", 503, "verifier_not_configured")
        })?;

        let verified = verifier.verify(&login.access_token).await.map_err(|_| {
            IdentityError::unavailable("/auth/login", 503, "token_verification_failed")
        })?;

        let row: Option<(String, String, String, String, i32)> = sqlx::query_as(
            "SELECT id::text, organization_id::text, name, status, version FROM users WHERE auth_subject = $1",
        )
        .bind(&verified.subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let Some((user_id, organization_id, name, status, version)) = row else {
            return Err(SessionError::UserNotBound);
        };

        if status == "DISABLED" {
            return Err(SessionError::UserDisabled);
        }

        if status != "ACTIVE" {
            return Err(IdentityError::SignIn.into());
        }

        self.create_session_for_user(
            &user_id,
            &organization_id,
            &name,
            version,
            login.refresh_token.as_deref(),
            now,
        )
        .await
    }

    pub async fn login(
        &self,
        identifier: &str,
        password: &str,
        client_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| IdentityError::unavailable("/api/auth/login", 503, "not_configured"))?;

        let login_resp = identity.login(identifier, password, client_ip).await?;
        self.sign_in_with_login(login_resp, now).await
    }

    pub async fn claim_invite(
        &self,
        identifier: &str,
        password: &str,
        invite_code: &str,
        client_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/invite/claim", 503, "not_configured")
        })?;
        let verifier = self.verifier.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/invite/claim", 503, "verifier_not_configured")
        })?;

        let login_resp = identity.login(identifier, password, client_ip).await?;
        let verified = verifier
            .verify(&login_resp.access_token)
            .await
            .map_err(|_| {
                IdentityError::unavailable("/auth/login", 503, "token_verification_failed")
            })?;

        let user_service = UserService::new(Arc::new(PgUserRepository::new()));
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        let claimed_user = user_service
            .claim(&mut tx, invite_code, &verified.subject, now)
            .await
            .map_err(|err| match err {
                ClaimError::Refused(reason) => SessionError::InviteRefused(reason),
                ClaimError::App(other) => SessionError::Database(other.to_string()),
            })?;

        let token = generate_session_token();
        let token_hash = sha256_hex(&token);
        let expires = now + Duration::seconds(SESSION_SECONDS);

        let refresh_enc = match (
            self.client_secret.as_deref(),
            login_resp.refresh_token.as_deref(),
        ) {
            (Some(secret), Some(rt)) => {
                let enc = encrypt_refresh_token(secret, rt).map_err(|_| {
                    SessionError::Identity(IdentityError::unavailable(
                        "/api/auth/invite/claim",
                        503,
                        "encrypt_failed",
                    ))
                })?;
                Some(enc)
            }
            _ => None,
        };

        sqlx::query(
            "INSERT INTO user_sessions (hash, user_id, version, expires, refresh_token_enc, organization_id) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid)",
        )
        .bind(&token_hash)
        .bind(&claimed_user.id)
        .bind(claimed_user.version)
        .bind(expires)
        .bind(refresh_enc)
        .bind(&claimed_user.organization_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        let roles = Self::fetch_user_roles(&self.pool, &claimed_user.id, now)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        Ok(SignInTail {
            user: SessionUser {
                id: claimed_user.id,
                name: claimed_user.name,
                organization_id: claimed_user.organization_id,
                roles,
            },
            session_token: token,
        })
    }

    pub async fn session_check(
        &self,
        session_token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<SessionUser>, SessionError> {
        let token_hash = sha256_hex(session_token);
        let row: Option<SessionCheckRow> = sqlx::query_as(
            "SELECT u.id::text, u.name, u.organization_id::text, u.status, u.version, s.version, s.expires \
             FROM user_sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let Some((user_id, name, org_id, status, u_version, s_version, expires)) = row else {
            return Ok(None);
        };

        if expires <= now || status != "ACTIVE" || u_version != s_version {
            return Ok(None);
        }

        let roles = Self::fetch_user_roles(&self.pool, &user_id, now)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        Ok(Some(SessionUser {
            id: user_id,
            name,
            organization_id: org_id,
            roles,
        }))
    }

    pub async fn refresh(
        &self,
        session_token: &str,
        client_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<SessionUser, SessionError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/refresh", 503, "not_configured")
        })?;
        let verifier = self.verifier.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/refresh", 503, "verifier_not_configured")
        })?;
        let secret = self.client_secret.as_deref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/refresh", 503, "client_secret_missing")
        })?;

        let token_hash = sha256_hex(session_token);
        let row: Option<RefreshSessionRow> = sqlx::query_as(
            "SELECT u.id::text, u.name, u.organization_id::text, u.auth_subject, u.status, u.version, s.version, s.expires, s.refresh_token_enc \
             FROM user_sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let Some((
            user_id,
            name,
            org_id,
            auth_subject,
            status,
            u_version,
            s_version,
            expires,
            refresh_enc,
        )) = row
        else {
            return Err(IdentityError::SignIn.into());
        };

        if expires <= now || status != "ACTIVE" || u_version != s_version {
            let _ = sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                .bind(&token_hash)
                .execute(&self.pool)
                .await;
            return Err(IdentityError::SignIn.into());
        }

        let Some(enc_bytes) = refresh_enc else {
            let _ = sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                .bind(&token_hash)
                .execute(&self.pool)
                .await;
            return Err(IdentityError::SignIn.into());
        };

        let decrypted_refresh = match decrypt_refresh_token(secret, &enc_bytes) {
            Ok(rt) => rt,
            Err(_) => {
                let _ = sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                    .bind(&token_hash)
                    .execute(&self.pool)
                    .await;
                return Err(IdentityError::SignIn.into());
            }
        };

        let login_resp = match identity.refresh(&decrypted_refresh, client_ip).await {
            Ok(resp) => resp,
            Err(e) => {
                let _ = sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                    .bind(&token_hash)
                    .execute(&self.pool)
                    .await;
                return Err(e.into());
            }
        };

        let verified = match verifier.verify(&login_resp.access_token).await {
            Ok(v) => v,
            Err(_) => {
                return Err(IdentityError::unavailable(
                    "/auth/refresh",
                    503,
                    "token_verification_failed",
                )
                .into());
            }
        };

        // Edge case (I, §9): a refresh whose new token names another sub -> session ends
        if Some(verified.subject) != auth_subject {
            let _ = sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
                .bind(&token_hash)
                .execute(&self.pool)
                .await;
            return Err(IdentityError::SignIn.into());
        }

        let new_refresh_token = login_resp
            .refresh_token
            .as_deref()
            .unwrap_or(&decrypted_refresh);
        let new_enc = encrypt_refresh_token(secret, new_refresh_token).map_err(|_| {
            SessionError::Identity(IdentityError::unavailable(
                "/api/auth/refresh",
                503,
                "encrypt_failed",
            ))
        })?;

        let new_expires = now + Duration::seconds(SESSION_SECONDS);
        sqlx::query(
            "UPDATE user_sessions SET expires = $1, refresh_token_enc = $2 WHERE hash = $3",
        )
        .bind(new_expires)
        .bind(new_enc)
        .bind(&token_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let roles = Self::fetch_user_roles(&self.pool, &user_id, now)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        Ok(SessionUser {
            id: user_id,
            name,
            organization_id: org_id,
            roles,
        })
    }

    pub async fn logout(
        &self,
        session_token: &str,
        client_ip: Option<&str>,
    ) -> Result<(), SessionError> {
        let token_hash = sha256_hex(session_token);
        let row: Option<Option<Vec<u8>>> = sqlx::query_scalar(
            "DELETE FROM user_sessions WHERE hash = $1 RETURNING refresh_token_enc",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        if let Some(Some(enc_bytes)) = row {
            if let Some(secret) = self.client_secret.as_deref() {
                if let Ok(plain_rt) = decrypt_refresh_token(secret, &enc_bytes) {
                    if let Some(identity) = self.identity.as_ref() {
                        let _ = identity.logout(&plain_rt, client_ip).await;
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn otp_request(
        &self,
        destination: &str,
        channel: &str,
        client_ip: Option<&str>,
    ) -> Result<IdentityOtpRequestResponse, SessionError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/otp/request", 503, "not_configured")
        })?;
        identity
            .otp_request(destination, channel, client_ip)
            .await
            .map_err(Into::into)
    }

    pub async fn otp_verify(
        &self,
        challenge_id: &str,
        code: &str,
        client_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/otp/verify", 503, "not_configured")
        })?;
        let resp = identity.otp_verify(challenge_id, code, client_ip).await?;
        if let Some(login) = resp.login {
            self.sign_in_with_login(login, now).await
        } else {
            Err(IdentityError::InvalidCode.into())
        }
    }

    pub async fn register(
        &self,
        phone: &str,
        password: &str,
        email: Option<&str>,
        client_ip: Option<&str>,
    ) -> Result<IdentityRegisterResponse, SessionError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/auth/register", 503, "not_configured")
        })?;
        identity
            .register(phone, password, email, client_ip)
            .await
            .map_err(Into::into)
    }

    pub async fn active_user_and_roles(
        &self,
        session_token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(User, Vec<ScopedRole>)>, SessionError> {
        let token_hash = sha256_hex(session_token);
        let row: Option<ActiveUserRow> = sqlx::query_as(
            "SELECT u.id::text, u.organization_id::text, u.auth_subject, u.name, u.status, u.version, s.version, s.expires \
             FROM user_sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let Some((id, org_id, auth_sub, name, status, u_version, s_version, expires)) = row else {
            return Ok(None);
        };

        if expires <= now || status != "ACTIVE" || u_version != s_version {
            return Ok(None);
        }

        let roles_rows: Vec<RoleRow> = sqlx::query_as(
            "SELECT role_code, branch_id, valid_from, valid_to FROM user_roles \
             WHERE user_id = $1::uuid AND valid_from <= $2 AND (valid_to IS NULL OR valid_to > $2) \
             ORDER BY valid_from ASC, id ASC",
        )
        .bind(&id)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let scoped_roles = roles_rows
            .into_iter()
            .filter_map(|(role_code, branch_id, valid_from, valid_to)| {
                let role = Role::parse(&role_code)?;
                let branch = branch_id.map(BranchId::new);
                Some(ScopedRole::new(role, branch, valid_from, valid_to))
            })
            .collect::<Vec<_>>();

        let user = User {
            id,
            organization_id: org_id,
            auth_subject: auth_sub,
            name,
            status: UserStatus::Active,
            version: u_version,
            created_at: now,
            created_by: "session".to_string(),
            legacy_employee_id: None,
            roles: scoped_roles.clone(),
        };

        Ok(Some((user, scoped_roles)))
    }

    pub async fn create_first_admin_invite(
        &self,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<String, String> {
        let owners: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT u.id::text, u.status, u.organization_id::text \
             FROM users u \
             WHERE u.legacy_employee_id IS NULL \
               AND EXISTS ( \
                   SELECT 1 FROM user_roles r \
                   WHERE r.user_id = u.id \
                     AND r.role_code = 'EXECUTIVE' \
                     AND r.branch_id IS NULL \
                     AND r.valid_from <= $1 \
                     AND (r.valid_to IS NULL OR r.valid_to > $1) \
               ) \
               AND EXISTS ( \
                   SELECT 1 FROM user_roles r \
                   WHERE r.user_id = u.id \
                     AND r.role_code = 'ADMIN' \
                     AND r.branch_id IS NULL \
                     AND r.valid_from <= $1 \
                     AND (r.valid_to IS NULL OR r.valid_to > $1) \
               )",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        match owners.len() {
            0 => {
                let org_id: String = sqlx::query_scalar(
                    "SELECT id::text FROM organizations WHERE code = 'XC' LIMIT 1",
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "organisation XC not found".to_string())?;

                let user_id = generate_uuid();
                let invite_code = generate_session_token();
                let code_hash = sha256_hex(&invite_code);
                let expires_at = now + Duration::days(7);

                let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;

                sqlx::query(
                    "INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by, legacy_employee_id) \
                     VALUES ($1::uuid, $2::uuid, NULL, $3, 'INVITED', 1, $4, 'create-admin-invite', NULL)",
                )
                .bind(&user_id)
                .bind(&org_id)
                .bind(name)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

                sqlx::query(
                    "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                     VALUES ($1::uuid, 'EXECUTIVE', $2::uuid, NULL, $3)",
                )
                .bind(&user_id)
                .bind(&org_id)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

                sqlx::query(
                    "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                     VALUES ($1::uuid, 'ADMIN', $2::uuid, NULL, $3)",
                )
                .bind(&user_id)
                .bind(&org_id)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

                sqlx::query(
                    "INSERT INTO user_invites (user_id, code_hash, expires_at, used_at) \
                     VALUES ($1::uuid, $2, $3, NULL)",
                )
                .bind(&user_id)
                .bind(&code_hash)
                .bind(expires_at)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

                tx.commit().await.map_err(|e| e.to_string())?;
                Ok(invite_code)
            }
            1 => {
                let (user_id, status, _) = &owners[0];
                if status == "INVITED" {
                    let invite_code = generate_session_token();
                    let code_hash = sha256_hex(&invite_code);
                    let expires_at = now + Duration::days(7);

                    sqlx::query(
                        "INSERT INTO user_invites (user_id, code_hash, expires_at, used_at) \
                         VALUES ($1::uuid, $2, $3, NULL) \
                         ON CONFLICT (user_id) DO UPDATE SET code_hash = EXCLUDED.code_hash, expires_at = EXCLUDED.expires_at, used_at = NULL",
                    )
                    .bind(user_id)
                    .bind(&code_hash)
                    .bind(expires_at)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| e.to_string())?;

                    Ok(invite_code)
                } else if status == "ACTIVE" {
                    Err("The owner user is already ACTIVE".to_string())
                } else {
                    Err(format!("Owner user has invalid status {status}"))
                }
            }
            _ => Err("More than one owner user exists".to_string()),
        }
    }
}
