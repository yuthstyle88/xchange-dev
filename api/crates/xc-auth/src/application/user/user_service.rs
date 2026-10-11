//! User application service.

use std::sync::Arc;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, PiiError, normalize};

use crate::domain::access::{Role, ScopeRule, ScopedRole};
use crate::domain::identity::identity_client::{
    IdentityClient, IdentityError, IdentityRegisterRequest,
};
use crate::domain::user::{User, UserRepository, UserStatus};
use crate::infrastructure::user::PgUserRepository;

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

/// What `create_staff` needs (#1157 §7). The password goes to Identity only; xchange never stores it.
#[derive(Debug, Clone)]
pub struct StaffCreate<'a> {
    pub name: &'a str,
    pub username: Option<&'a str>,
    pub phone: &'a str,
    pub email: Option<&'a str>,
    pub password: &'a str,
    pub roles: Vec<ScopedRole>,
    /// Identity's `must_change_password`: false for `userCreate`, true for `create-owner`.
    pub must_change_password: bool,
    /// Complete an existing `PENDING` user (the retry after Identity refused or was down).
    pub retry_user_id: Option<&'a str>,
    pub created_by: &'a str,
    /// The creator's roles: each role the new user gets must be inside the creator's own ADMIN scope.
    /// `None` only for `create-owner`, which runs on the server with no signed-in creator.
    pub creator_roles: Option<&'a [ScopedRole]>,
}

/// Whether a creator holding `creator` may give every role in `wanted` (#1157 review): an
/// organisation-wide role needs ADMIN organisation-wide; a branch role needs ADMIN organisation-wide
/// or ADMIN on that same branch. Only roles valid at `now` count.
pub fn creator_may_grant(
    creator: &[ScopedRole],
    wanted: &[ScopedRole],
    now: DateTime<Utc>,
) -> bool {
    let admin_scopes: Vec<Option<&BranchId>> = creator
        .iter()
        .filter(|r| {
            r.role == Role::Admin && r.valid_from <= now && r.valid_to.is_none_or(|to| to > now)
        })
        .map(|r| r.branch.as_ref())
        .collect();
    let org_wide = admin_scopes.iter().any(Option::is_none);
    wanted.iter().all(|w| match &w.branch {
        None => org_wide,
        Some(b) => org_wide || admin_scopes.contains(&Some(b)),
    })
}

/// Why a staff create is refused. Nothing is bound in xchange for any of these.
#[derive(Debug)]
pub enum CreateStaffError {
    /// A field is missing or wrong (`validationFailed` on that field).
    Validation {
        field: &'static str,
        message: String,
    },
    /// Identity refused or is down; the user row stays `PENDING`.
    Identity(IdentityError),
    /// Identity made the account but the bind failed; `identity_id` is in `user_audit`.
    BindFailed {
        user_id: String,
        identity_id: String,
    },
    /// A role asked for is outside the creator's own ADMIN scope; nothing is written, Identity is not called.
    OutOfScope,
    App(AppError),
}

impl From<AppError> for CreateStaffError {
    fn from(e: AppError) -> Self {
        Self::App(e)
    }
}

fn identity_phone(raw: &str) -> Result<String, CreateStaffError> {
    normalize(IdentityType::Phone, raw, None).map_err(|e| match e {
        PiiError::Empty => CreateStaffError::Validation {
            field: "phone",
            message: "กรุณากรอกเบอร์โทรศัพท์".to_string(),
        },
        _ => CreateStaffError::Validation {
            field: "phone",
            message: "เบอร์โทรศัพท์ไม่ถูกต้อง".to_string(),
        },
    })
}

impl<R: UserRepository> UserService<R> {
    /// Creates a staff member and their Identity account (#1157, design §5.3):
    /// 1. insert `users` as `PENDING` with its roles, and commit;
    /// 2. call Identity register outside any transaction;
    /// 3. in a new transaction, bind `auth_subject` and set `ACTIVE`.
    ///
    /// Identity refused or down → the row stays `PENDING` (retry with `retry_user_id`).
    /// Registered but the bind fails → the returned id is written to `user_audit` and returned.
    pub async fn create_staff(
        &self,
        pool: &PgPool,
        identity: &dyn IdentityClient,
        organization_id: &str,
        input: &StaffCreate<'_>,
        now: DateTime<Utc>,
    ) -> Result<User, CreateStaffError> {
        if input.name.trim().is_empty() {
            return Err(CreateStaffError::Validation {
                field: "name",
                message: "กรุณากรอกชื่อ".to_string(),
            });
        }
        let phone = identity_phone(input.phone)?;
        if input.password.is_empty() {
            return Err(CreateStaffError::Validation {
                field: "password",
                message: "กรุณากรอกรหัสผ่าน".to_string(),
            });
        }

        // 1. the PENDING row and its roles, committed before Identity is called.
        let mut tx = pool.begin().await.map_err(AppError::Database)?;
        let mut user = match input.retry_user_id {
            Some(id) => {
                let user = self.repo.find_by_id(&mut tx, id).await?.ok_or_else(|| {
                    CreateStaffError::Validation {
                        field: "userId",
                        message: "ไม่พบผู้ใช้งาน".to_string(),
                    }
                })?;
                if user.organization_id != organization_id || user.status != UserStatus::Pending {
                    return Err(CreateStaffError::Validation {
                        field: "userId",
                        message: "ผู้ใช้นี้สร้างเสร็จแล้ว".to_string(),
                    });
                }
                if let Some(creator) = input.creator_roles
                    && !creator_may_grant(creator, &user.roles, now)
                {
                    return Err(CreateStaffError::OutOfScope);
                }
                user
            }
            None => {
                if let Some(creator) = input.creator_roles
                    && !creator_may_grant(creator, &input.roles, now)
                {
                    return Err(CreateStaffError::OutOfScope);
                }
                let mut accumulated: Vec<ScopedRole> = Vec::new();
                for r in &input.roles {
                    ScopeRule::check(r.role, r.branch.as_ref(), &accumulated).map_err(|e| {
                        CreateStaffError::Validation {
                            field: "roles",
                            message: format!("Scope rule rejected role {:?}: {e}", r.role),
                        }
                    })?;
                    accumulated.push(r.clone());
                }
                let user = User {
                    id: generate_uuid(),
                    organization_id: organization_id.to_string(),
                    auth_subject: None,
                    name: input.name.trim().to_string(),
                    status: UserStatus::Pending,
                    version: 1,
                    created_at: now,
                    created_by: input.created_by.to_string(),
                    legacy_employee_id: None,
                    roles: input.roles.clone(),
                };
                self.repo.create_user(&mut tx, &user).await?;
                for r in &input.roles {
                    self.repo
                        .add_role(&mut tx, &user.id, organization_id, r)
                        .await?;
                }
                self.repo
                    .append_audit(
                        &mut tx,
                        input.created_by,
                        &user.id,
                        "userCreate",
                        organization_id,
                    )
                    .await?;
                user
            }
        };
        tx.commit().await.map_err(AppError::Database)?;

        // 2. Identity, outside any transaction.
        let registered = identity
            .register(&IdentityRegisterRequest {
                username: input.username,
                phone: &phone,
                email: input.email,
                password: input.password,
                must_change_password: input.must_change_password,
            })
            .await
            .map_err(CreateStaffError::Identity)?;
        let identity_id = registered.identity_id.ok_or_else(|| {
            CreateStaffError::Identity(IdentityError::unavailable(
                "/auth/register",
                201,
                "no_identity_id",
            ))
        })?;

        // 3. bind and activate in a new transaction; a failed bind never loses the identity id.
        let bound: AppResult<()> = async {
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            if self
                .repo
                .find_by_auth_subject(&mut tx, &identity_id)
                .await?
                .is_some()
            {
                return Err(AppError::Domain(
                    "identity already bound to another user".to_string(),
                ));
            }
            self.repo
                .set_auth_subject(&mut tx, &user.id, &identity_id)
                .await?;
            self.repo
                .set_status(&mut tx, &user.id, UserStatus::Active)
                .await?;
            tx.commit().await.map_err(AppError::Database)?;
            Ok(())
        }
        .await;

        if bound.is_err() {
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            self.repo
                .append_audit(
                    &mut tx,
                    input.created_by,
                    &user.id,
                    &format!("userCreateBindFailed:{identity_id}"),
                    organization_id,
                )
                .await?;
            tx.commit().await.map_err(AppError::Database)?;
            return Err(CreateStaffError::BindFailed {
                user_id: user.id,
                identity_id,
            });
        }

        user.auth_subject = Some(identity_id);
        user.status = UserStatus::Active;
        user.version += 1;
        Ok(user)
    }
}
