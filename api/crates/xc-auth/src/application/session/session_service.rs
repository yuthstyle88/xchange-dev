use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, normalize};

use crate::application::user::UserService;
use crate::application::user::user_service::{CreateStaffError, StaffCreate};
use crate::domain::access::{BranchRef, Role, ScopedRole, SessionBranch};
use crate::domain::identity::identity_client::{
    IdentityClient, IdentityError, IdentityLoginResponse, IdentityOtpRequestResponse,
    UNAVAILABLE_MESSAGE,
};
use crate::domain::token::{TokenVerifier, VerifiedToken};
use crate::domain::user::{User, UserStatus};
use crate::infrastructure::user::PgUserRepository;

use super::login_recorder::{LoginAttempt, LoginRecorder, LoginResult};
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
    /// `user_sessions.must_change_password` (design §3.6 F11, BE ruling #564 issuecomment-6245742828 (a)): true
    /// until the person changes the temporary password. RBAC F11a sets it on the login and session answers.
    #[serde(rename = "mustChangePassword", default)]
    pub must_change_password: bool,
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
                "บัญชีนี้ยังไม่มีผู้ใช้ใน XCHANGE ให้เจ้าของร้านสร้างบัญชีให้".to_string(),
            ),
            Self::Database(_) => (503, "identityUnavailable", UNAVAILABLE_MESSAGE.to_string()),
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
    Option<String>,
    Option<String>,
    String,
);
/// `users.last_branch_id` and the organisation's branches (`ORG_BRANCHES_JSON`), read at sign-in.
type SignInBranchRow = (Option<String>, String);
type RoleRow = (String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>);

/// The organisation's branches as one JSON text, `[{id, code, name}]`, for a row aliased `u` of `users` (SB-1
/// §14.3). A sub-select, so it folds into the statement that reads the user: no extra query. `code` is
/// `COALESCE(code, id)` for a branch inserted without one.
const ORG_BRANCHES_JSON: &str = "(SELECT COALESCE(json_agg(json_build_object('id', b.id, 'code', COALESCE(b.code, b.id), \
     'name', b.name)), '[]'::json)::text FROM branches b WHERE b.organization_id = u.organization_id)";

#[derive(Deserialize)]
struct BranchJson {
    id: String,
    code: String,
    name: String,
}

fn branch_refs(json: &str) -> Result<Vec<BranchRef>, SessionError> {
    let rows: Vec<BranchJson> =
        serde_json::from_str(json).map_err(|e| SessionError::Database(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|b| BranchRef::new(&b.id, &b.code, &b.name))
        .collect())
}

fn scoped_roles(rows: Vec<RoleRow>) -> Vec<ScopedRole> {
    rows.into_iter()
        .filter_map(|(role_code, branch_id, valid_from, valid_to)| {
            let role = Role::parse(&role_code)?;
            Some(ScopedRole::new(
                role,
                branch_id.map(BranchId::new),
                valid_from,
                valid_to,
            ))
        })
        .collect()
}

const VALID_ROLES_SQL: &str = "SELECT role_code, branch_id, valid_from, valid_to FROM user_roles \
     WHERE user_id = $1::uuid AND valid_from <= $2 AND (valid_to IS NULL OR valid_to > $2) \
     ORDER BY valid_from ASC, id ASC";

/// One active session as a read answers it (SB-1 §14.3): the user, their valid roles, the session's stored branch,
/// the user's last choice, and the organisation's branches. Read in two statements, as before §14.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    pub user: User,
    pub roles: Vec<ScopedRole>,
    /// `user_sessions.branch_id`: `None` for a session opened before the migration.
    pub stored_branch: Option<BranchId>,
    /// `users.last_branch_id`.
    pub last_branch: Option<BranchId>,
    pub org_branches: Vec<BranchRef>,
}

impl ActiveSession {
    /// S: the branches this user may switch to now (§14.2).
    pub fn switchable(&self, now: DateTime<Utc>) -> Vec<BranchRef> {
        SessionBranch::switchable(&self.roles, &self.org_branches, now)
    }

    /// The branch this session works in now: the stored one while it is in S, else R5. Never written back.
    pub fn effective_branch(&self, now: DateTime<Utc>) -> Option<BranchId> {
        SessionBranch::effective(
            self.stored_branch.as_ref(),
            &self.switchable(now),
            self.last_branch.as_ref(),
        )
    }
}

/// Why `switch_branch` refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchBranchError {
    /// No active session for the token.
    SignIn,
    /// The branch is not in S (not granted, unknown, or another organisation's).
    OutOfScope,
    Database(String),
}

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

    async fn fetch_user_roles<'e, E>(
        executor: E,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<SessionUserRole>, sqlx::Error>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT role_code, branch_id FROM user_roles \
             WHERE user_id = $1::uuid AND valid_from <= $2 AND (valid_to IS NULL OR valid_to > $2) \
             ORDER BY valid_from ASC, id ASC",
        )
        .bind(user_id)
        .bind(now)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(role, branch_id)| SessionUserRole { role, branch_id })
            .collect())
    }

    /// R5 at sign-in: the branch a new session of `user_id` starts on, from its valid roles, the organisation's
    /// branches and the user's last choice. Sign-in is not a choice: `last_branch_id` is only read.
    async fn initial_branch_in(
        conn: &mut sqlx::PgConnection,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<BranchId>, SessionError> {
        let sql = format!(
            "SELECT u.last_branch_id, {ORG_BRANCHES_JSON} FROM users u WHERE u.id = $1::uuid"
        );
        let row: Option<SignInBranchRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        let Some((last, branches)) = row else {
            return Ok(None);
        };
        let roles: Vec<RoleRow> = sqlx::query_as(VALID_ROLES_SQL)
            .bind(user_id)
            .bind(now)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        let s = SessionBranch::switchable(&scoped_roles(roles), &branch_refs(&branches)?, now);
        Ok(SessionBranch::initial(&s, last.map(BranchId::new).as_ref()))
    }

    async fn create_session_for_user(
        &self,
        user_id: &str,
        organization_id: &str,
        name: &str,
        version: i32,
        login: &IdentityLoginResponse,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        let token = generate_session_token();
        let token_hash = sha256_hex(&token);
        let expires = now + Duration::seconds(SESSION_SECONDS);
        let must_change_password = login.must_change_password.unwrap_or(false);

        let refresh_enc = match (
            self.client_secret.as_deref(),
            login.refresh_token.as_deref(),
        ) {
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

        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        let branch = Self::initial_branch_in(&mut conn, user_id, now).await?;
        sqlx::query(
            "INSERT INTO user_sessions (hash, user_id, version, expires, refresh_token_enc, organization_id, branch_id, must_change_password) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6::uuid, $7, $8)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(version)
        .bind(expires)
        .bind(refresh_enc)
        .bind(organization_id)
        .bind(branch.as_ref().map(BranchId::as_str))
        .bind(must_change_password)
        .execute(&mut *conn)
        .await
        .map_err(|e| SessionError::Database(e.to_string()))?;

        let roles = Self::fetch_user_roles(&mut *conn, user_id, now)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        Ok(SignInTail {
            user: SessionUser {
                id: user_id.to_string(),
                name: name.to_string(),
                organization_id: organization_id.to_string(),
                roles,
                must_change_password: false, // RBAC F11a sets the real value
            },
            session_token: token,
        })
    }

    pub async fn sign_in_with_login(
        &self,
        login: IdentityLoginResponse,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, SessionError> {
        self.sign_in_with_login_inner(login, now)
            .await
            .map_err(|(e, _)| e)
    }

    async fn sign_in_with_login_inner(
        &self,
        login: IdentityLoginResponse,
        now: DateTime<Utc>,
    ) -> Result<SignInTail, (SessionError, Option<(String, String)>)> {
        let verifier = self.verifier.as_ref().ok_or_else(|| {
            (
                IdentityError::unavailable("/auth/login", 503, "verifier_not_configured").into(),
                None,
            )
        })?;

        let verified = verifier.verify(&login.access_token).await.map_err(|_| {
            (
                IdentityError::unavailable("/auth/login", 503, "token_verification_failed").into(),
                None,
            )
        })?;

        let row: Option<(String, String, String, String, i32)> = sqlx::query_as(
            "SELECT id::text, organization_id::text, name, status, version FROM users WHERE auth_subject = $1",
        )
        .bind(&verified.subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| (SessionError::Database(e.to_string()), None))?;

        let Some((user_id, organization_id, name, status, version)) = row else {
            return Err((SessionError::UserNotBound, None));
        };

        if status == "DISABLED" {
            return Err((SessionError::UserDisabled, Some((user_id, organization_id))));
        }

        if status != "ACTIVE" {
            return Err((
                IdentityError::SignIn.into(),
                Some((user_id, organization_id)),
            ));
        }

        self.create_session_for_user(&user_id, &organization_id, &name, version, &login, now)
            .await
            .map_err(|e| (e, None))
    }

    fn identity_identifier(raw: &str) -> String {
        let stripped: String = raw
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '(' && *c != ')')
            .collect();

        let is_phone = if let Some(rest) = stripped.strip_prefix('0') {
            (rest.len() == 8 || rest.len() == 9) && rest.chars().all(|c| c.is_ascii_digit())
        } else if let Some(rest) = stripped.strip_prefix('+') {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        } else {
            false
        };

        if is_phone {
            normalize(IdentityType::Phone, raw, None).unwrap_or_else(|_| raw.to_string())
        } else {
            raw.to_string()
        }
    }

    /// Identity's login for `identifier` / `password`, with its access token verified, and no user looked up or
    /// session made (#654 CI-7: customer register binds the subject first, then signs in with
    /// [`Self::sign_in_with_login`]). A token the verifier refuses is `identityUnavailable`, as at sign-in.
    pub async fn verified_login(
        &self,
        identifier: &str,
        password: &str,
        client_ip: Option<&str>,
    ) -> Result<(IdentityLoginResponse, VerifiedToken), SessionError> {
        let identity = self.identity.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/api/app/register", 503, "not_configured")
        })?;
        let verifier = self.verifier.as_ref().ok_or_else(|| {
            IdentityError::unavailable("/auth/login", 503, "verifier_not_configured")
        })?;
        let login = identity
            .login(&Self::identity_identifier(identifier), password, client_ip)
            .await?;
        let verified = verifier.verify(&login.access_token).await.map_err(|_| {
            IdentityError::unavailable("/auth/login", 503, "token_verification_failed")
        })?;
        Ok((login, verified))
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

        let id = Self::identity_identifier(identifier);
        let outcome = match identity.login(&id, password, client_ip).await {
            Ok(login_resp) => self.sign_in_with_login_inner(login_resp, now).await,
            Err(err) => Err((err.into(), None)),
        };

        let attempt = match &outcome {
            Ok(tail) => LoginAttempt {
                identifier_sent: &id,
                ip: client_ip,
                result: LoginResult::Ok,
                user_id: Some(&tail.user.id),
                organization_id: Some(&tail.user.organization_id),
            },
            Err((err, user_org)) => {
                let (user_id, organization_id) = match user_org {
                    Some((u, o)) => (Some(u.as_str()), Some(o.as_str())),
                    None => (None, None),
                };
                LoginAttempt {
                    identifier_sent: &id,
                    ip: client_ip,
                    result: match err {
                        SessionError::Identity(IdentityError::InvalidCredentials) => {
                            LoginResult::BadPassword
                        }
                        SessionError::Identity(
                            IdentityError::TooManyAttempts | IdentityError::AccountLocked,
                        ) => LoginResult::RateLimited,
                        SessionError::Identity(IdentityError::IdentityDisabled) => {
                            LoginResult::NotActive
                        }
                        SessionError::UserNotBound => LoginResult::UnknownUser,
                        SessionError::UserDisabled => LoginResult::NotActive,
                        SessionError::Identity(IdentityError::SignIn) if user_org.is_some() => {
                            LoginResult::NotActive
                        }
                        _ => LoginResult::Unavailable,
                    },
                    user_id,
                    organization_id,
                }
            }
        };

        if let Err(e) = LoginRecorder::new(&self.pool).record(attempt).await {
            eprintln!("login_history write failed: {e}");
        }

        outcome.map_err(|(e, _)| e)
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
            must_change_password: false, // RBAC F11a sets the real value
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
            Err(e @ IdentityError::Unavailable { .. }) => return Err(e.into()),
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
            must_change_password: false, // RBAC F11a sets the real value
        })
    }

    /// `POST /api/auth/password` (design §3.6 F11; RBAC F11a fills the body, F11b calls it from `http/auth.rs`):
    /// reads the session by `session_token`, decrypts its refresh token, gets an access token with
    /// `IdentityClient::refresh`, then `IdentityClient::change_password(access_token, current, new, client_ip)`.
    /// On success the session row is deleted, so the session ends (Identity revokes all its tokens), and the caller
    /// clears the cookie as logout does. Identity's refusals come back as `SessionError::Identity`.
    pub async fn change_password(
        &self,
        session_token: &str,
        current_password: &str,
        new_password: &str,
        client_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), SessionError> {
        let _ = (
            session_token,
            current_password,
            new_password,
            client_ip,
            now,
        );
        todo!("RBAC F11a: SessionService::change_password")
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

    /// The session read (SB-1 §14.3) on `conn`: exactly two statements, as before §14. The first reads the session,
    /// the user, the stored branch, the last choice and the organisation's branches; the second reads the valid roles.
    async fn read_session_in(
        conn: &mut sqlx::PgConnection,
        session_token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ActiveSession>, SessionError> {
        let token_hash = sha256_hex(session_token);
        let sql = format!(
            "SELECT u.id::text, u.organization_id::text, u.auth_subject, u.name, u.status, u.version, s.version, \
             s.expires, s.branch_id, u.last_branch_id, {ORG_BRANCHES_JSON} \
             FROM user_sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.hash = $1"
        );
        let row: Option<ActiveUserRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(&token_hash)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;

        let Some((
            id,
            org_id,
            auth_sub,
            name,
            status,
            u_version,
            s_version,
            expires,
            stored_branch,
            last_branch,
            org_branches,
        )) = row
        else {
            return Ok(None);
        };

        if expires <= now || status != "ACTIVE" || u_version != s_version {
            return Ok(None);
        }

        let roles_rows: Vec<RoleRow> = sqlx::query_as(VALID_ROLES_SQL)
            .bind(&id)
            .bind(now)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        let roles = scoped_roles(roles_rows);

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
            roles: roles.clone(),
        };

        Ok(Some(ActiveSession {
            user,
            roles,
            stored_branch: stored_branch.map(BranchId::new),
            last_branch: last_branch.map(BranchId::new),
            org_branches: branch_refs(&org_branches)?,
        }))
    }

    /// The active session behind `session_token`, or `None` (SB-1 §14.3). Reading never writes the session row.
    pub async fn active_session(
        &self,
        session_token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ActiveSession>, SessionError> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        Self::read_session_in(&mut conn, session_token, now).await
    }

    /// The user and roles of an active session, for callers that need no branch.
    pub async fn active_user_and_roles(
        &self,
        session_token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(User, Vec<ScopedRole>)>, SessionError> {
        Ok(self
            .active_session(session_token, now)
            .await?
            .map(|s| (s.user, s.roles)))
    }

    /// `POST /api/auth/session/branch` (SB-1 §14.3): moves this session to `branch` and records it as the user's
    /// last choice, in one transaction. A branch outside S is refused and nothing is written. `users.version` is
    /// not bumped: that would end every session of the user.
    pub async fn switch_branch(
        &self,
        session_token: &str,
        branch: &str,
        now: DateTime<Utc>,
    ) -> Result<ActiveSession, SwitchBranchError> {
        let db = |e: sqlx::Error| SwitchBranchError::Database(e.to_string());
        let mut tx = self.pool.begin().await.map_err(db)?;
        let mut session = match Self::read_session_in(&mut tx, session_token, now).await {
            Ok(Some(session)) => session,
            Ok(None) => return Err(SwitchBranchError::SignIn),
            Err(SessionError::Database(e)) => return Err(SwitchBranchError::Database(e)),
            Err(e) => return Err(SwitchBranchError::Database(e.refusal().2)),
        };
        let chosen = BranchId::new(branch);
        if !session.switchable(now).iter().any(|b| b.id == chosen) {
            return Err(SwitchBranchError::OutOfScope);
        }
        sqlx::query("UPDATE user_sessions SET branch_id = $2 WHERE hash = $1")
            .bind(sha256_hex(session_token))
            .bind(chosen.as_str())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("UPDATE users SET last_branch_id = $2 WHERE id = $1::uuid")
            .bind(&session.user.id)
            .bind(chosen.as_str())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        session.stored_branch = Some(chosen.clone());
        session.last_branch = Some(chosen);
        Ok(session)
    }

    /// S for the user `user_id` (SB-1 §14.4 `Me.branches`): their valid-now roles over their organisation's
    /// branches, ordered as §14.2. `None` when no such user.
    pub async fn switchable_of_user(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Vec<BranchRef>>, SessionError> {
        // The old owner and staff cookies act under ids that are not user ids ("owner", an employee id).
        let is_uuid = user_id.len() == 36
            && user_id.char_indices().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == '-',
                _ => c.is_ascii_hexdigit(),
            });
        if !is_uuid {
            return Ok(None);
        }
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        let sql = format!(
            "SELECT u.last_branch_id, {ORG_BRANCHES_JSON} FROM users u WHERE u.id = $1::uuid"
        );
        let row: Option<SignInBranchRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        let Some((_, branches)) = row else {
            return Ok(None);
        };
        let roles: Vec<RoleRow> = sqlx::query_as(VALID_ROLES_SQL)
            .bind(user_id)
            .bind(now)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| SessionError::Database(e.to_string()))?;
        Ok(Some(SessionBranch::switchable(
            &scoped_roles(roles),
            &branch_refs(&branches)?,
            now,
        )))
    }

    /// The first owner of an install (#1157, the first-user bootstrap): one user with
    /// organisation-wide EXECUTIVE + ADMIN, whose Identity account is created server-side with
    /// `must_change_password: true`, bound and `ACTIVE`. An owner left `PENDING` (Identity refused or
    /// was down) is completed by running it again; an `ACTIVE` owner refuses.
    pub async fn create_owner(
        &self,
        name: &str,
        phone: &str,
        password: &str,
        now: DateTime<Utc>,
    ) -> Result<String, String> {
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| "Identity is not configured (IDENTITY_BASE_URL)".to_string())?;

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

        let (org_id, retry_user_id) = match owners.as_slice() {
            [] => {
                let org_id: String = sqlx::query_scalar(
                    "SELECT id::text FROM organizations WHERE code = 'XC' LIMIT 1",
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "organisation XC not found".to_string())?;
                (org_id, None)
            }
            [(user_id, status, org_id)] => match status.as_str() {
                "PENDING" => (org_id.clone(), Some(user_id.clone())),
                "ACTIVE" => return Err("The owner user is already ACTIVE".to_string()),
                other => return Err(format!("Owner user has invalid status {other}")),
            },
            _ => return Err("More than one owner user exists".to_string()),
        };

        // A PENDING owner (the users migration's mirror of the owner account, or an earlier run that
        // Identity refused) takes the name this command gives it.
        if let (Some(user_id), false) = (&retry_user_id, name.trim().is_empty()) {
            sqlx::query("UPDATE users SET name = $1 WHERE id = $2::uuid AND status = 'PENDING'")
                .bind(name.trim())
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
        }

        let users = UserService::new(Arc::new(PgUserRepository::new()));
        let input = StaffCreate {
            name,
            username: None,
            phone,
            email: None,
            password,
            roles: vec![
                ScopedRole::new(Role::Executive, None, now, None),
                ScopedRole::new(Role::Admin, None, now, None),
            ],
            must_change_password: true,
            retry_user_id: retry_user_id.as_deref(),
            created_by: "create-owner",
            creator_roles: None,
        };
        match users
            .create_staff(&self.pool, identity.as_ref(), &org_id, &input, now)
            .await
        {
            Ok(user) => Ok(user.id),
            Err(CreateStaffError::Validation { field, message }) => {
                Err(format!("{field}: {message}"))
            }
            // create-owner passes no creator roles, so the scope rule never refuses it.
            Err(CreateStaffError::OutOfScope) => {
                Err("roles outside the creator's scope".to_string())
            }
            Err(CreateStaffError::Identity(e)) => {
                let (_, code, message) = e.refusal();
                Err(format!(
                    "Identity refused ({code}): {message}; the owner stays PENDING, run again"
                ))
            }
            Err(CreateStaffError::BindFailed {
                user_id,
                identity_id,
            }) => Err(format!(
                "Identity account {identity_id} was created but could not be bound to user {user_id} \
                 (recorded in user_audit)"
            )),
            Err(CreateStaffError::App(e)) => Err(e.to_string()),
        }
    }
}
