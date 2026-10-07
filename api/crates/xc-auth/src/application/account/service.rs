//! Account service application layer.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use xc_kernel::ids::BranchId;

use crate::domain::account::entities::employee::Employee;
use crate::domain::account::entities::owner::Owner;
use crate::domain::account::entities::session::{Login, SignedIn};
use crate::domain::account::repositories::account_repository::{AccountRepository, Purged};
use crate::domain::account::value_objects::password_hasher::PasswordHasher;

const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
const LIMIT: &str = "ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที";
const EMPLOYEE_DATA: &str = "ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง";
const NOT_FOUND: &str = "ไม่พบพนักงาน";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const MUST_CHANGE: &str = "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน";

const ROLES: &[&str] = &[
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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

fn is_valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The password a payload sets. Anything that is not a JSON string reads as
/// the empty password, which `PasswordHasher::hash` refuses with the length
/// sentence — the one place that rule is written.
fn new_password(payload: &Value) -> &str {
    payload
        .get("password")
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let millis = now.timestamp_millis();
    let window_millis = millis.div_euclid(15 * 60 * 1000) * (15 * 60 * 1000);
    DateTime::from_timestamp_millis(window_millis).unwrap_or(now)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmployeesAnswer {
    List(Vec<Employee>),
    Saved,
}

pub struct AccountService<R> {
    repo: Arc<R>,
    hasher: PasswordHasher,
    /// Made on first use by `hasher`, shared by clones. See [`Self::spare_hash`].
    spare: Arc<OnceLock<String>>,
}

// Not derived: a derive would ask for `R: Clone`, and only the `Arc` is cloned.
impl<R> Clone for AccountService<R> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            hasher: self.hasher.clone(),
            spare: Arc::clone(&self.spare),
        }
    }
}

impl<R: AccountRepository> AccountService<R> {
    pub fn new(repo: Arc<R>, hasher: PasswordHasher) -> Self {
        Self {
            repo,
            hasher,
            spare: Arc::new(OnceLock::new()),
        }
    }

    /// The PHC string a sign-in is verified against when there is nothing else
    /// to verify (no usable account, or a password field that cannot be a
    /// password). It is the hash of 32 random bytes, made once by this
    /// service's own hasher, so it costs what a real account costs.
    pub fn spare_hash(&self) -> &str {
        self.spare
            .get_or_init(|| self.hasher.hash(&generate_token()).unwrap_or_default())
    }

    /// The password step of every sign-in: exactly one verification at the
    /// hasher's cost, whatever the account (`stored` is `None` when there is
    /// no usable one) and whatever the payload's password field holds.
    fn password_matches(&self, payload: &Value, stored: Option<&str>) -> bool {
        let password = payload.get("password").and_then(|v| v.as_str());
        self.hasher
            .verify_at_full_cost(password, stored, self.spare_hash())
    }

    /// The shared door (`/api/auth` login): the owner if the username is an owner's, else an employee.
    pub async fn login(&self, payload: &Value, ip: &str, now: DateTime<Utc>) -> AppResult<Login> {
        let raw_username = payload
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let username = raw_username.trim().to_lowercase();
        let owner_opt = if !username.is_empty() {
            self.repo.find_owner_by_username(&username).await?
        } else {
            None
        };

        match owner_opt {
            Some((owner, password_hash)) => {
                let win = window_start(now);
                // Address counter
                let ip_key = sha256_hex(&format!("IP:{ip}"));
                let ip_count = self.repo.increment_attempt(&ip_key, win).await?;
                if ip_count > 30 {
                    return Err(AppError::Domain(LIMIT.to_string()));
                }

                // Name counter
                let name_key = sha256_hex(&format!("OWNER:{username}"));
                let name_count = self.repo.increment_attempt(&name_key, win).await?;
                if name_count > 10 {
                    return Err(AppError::Domain(LIMIT.to_string()));
                }

                // Password check
                let valid_pwd = self.password_matches(payload, Some(&password_hash));

                if !owner.active || !valid_pwd {
                    return Err(AppError::Domain(WRONG.to_string()));
                }

                let token = generate_token();
                let token_hash = sha256_hex(&token);
                let expires = now + chrono::Duration::hours(12);
                self.repo
                    .create_owner_session(&token_hash, &owner.id, owner.version, expires)
                    .await?;
                Ok(Login {
                    token,
                    who: SignedIn::Owner(owner),
                })
            }
            None => self.login_employee(payload, ip, now).await,
        }
    }

    /// The staff door (`/api/staff/login`): employees only.
    pub async fn login_employee(
        &self,
        payload: &Value,
        ip: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Login> {
        let raw_username = payload
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let username = raw_username.trim().to_lowercase();

        // (1) staff path only: a username longer than 40 UTF-16 units -> WRONG, nothing counted
        if username.encode_utf16().count() > 40 {
            return Err(AppError::Domain(WRONG.to_string()));
        }

        let win = window_start(now);

        // (2) address counter + 1, refused with LIMIT if now above 30
        let ip_key = sha256_hex(&format!("IP:{ip}"));
        let ip_count = self.repo.increment_attempt(&ip_key, win).await?;
        if ip_count > 30 {
            return Err(AppError::Domain(LIMIT.to_string()));
        }

        // (3) name counter + 1, refused with LIMIT if above 10
        let name_key = sha256_hex(&format!("USER:{username}"));
        let name_count = self.repo.increment_attempt(&name_key, win).await?;
        if name_count > 10 {
            return Err(AppError::Domain(LIMIT.to_string()));
        }

        // (4) account looked up and password checked
        let emp_opt = if !username.is_empty() {
            self.repo.find_employee_by_username(&username).await?
        } else {
            None
        };

        // No such name and a disabled account are "no usable account": the
        // same answer after the same work as a wrong password.
        let usable = emp_opt.filter(|(employee, _)| employee.active);
        let stored = usable
            .as_ref()
            .map(|(_, password_hash)| password_hash.as_str());
        if !self.password_matches(payload, stored) {
            return Err(AppError::Domain(WRONG.to_string()));
        }
        // A match implies an account; this arm is never taken.
        let Some((employee, _)) = usable else {
            return Err(AppError::Domain(WRONG.to_string()));
        };

        let token = generate_token();
        let token_hash = sha256_hex(&token);
        let expires = now + chrono::Duration::hours(12);
        self.repo
            .create_employee_session(&token_hash, &employee.id, employee.version, expires)
            .await?;
        self.repo
            .record_employee_login_audit(&employee.id, now)
            .await?;

        Ok(Login {
            token,
            who: SignedIn::Employee(employee),
        })
    }

    /// Who the two cookies are. "" = no cookie.
    pub async fn status(
        &self,
        owner_token: &str,
        staff_token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<SignedIn>> {
        if !owner_token.is_empty() && is_valid_token(owner_token) {
            let token_hash = sha256_hex(owner_token);
            if let Some((owner, session_version, expires)) =
                self.repo.find_owner_session(&token_hash).await?
            {
                if expires > now && session_version == owner.version && owner.active {
                    return Ok(Some(SignedIn::Owner(owner)));
                }
            }
        }

        if !staff_token.is_empty() {
            return match self.employee_session(staff_token, false, now).await {
                Ok(employee) => Ok(Some(SignedIn::Employee(employee))),
                // No usable session, or a first password still to change: nobody is signed in.
                Err(AppError::Unauthenticated | AppError::Domain(_)) => Ok(None),
                // Anything else (the database) is not an answer about the session.
                Err(other) => Err(other),
            };
        }

        Ok(None)
    }

    /// The employee of a staff session. `allow_must_change` lets a first-password session through.
    pub async fn employee_session(
        &self,
        token: &str,
        allow_must_change: bool,
        now: DateTime<Utc>,
    ) -> AppResult<Employee> {
        if !is_valid_token(token) {
            return Err(AppError::Unauthenticated);
        }

        let token_hash = sha256_hex(token);
        let (employee, session_version, expires) =
            match self.repo.find_employee_session(&token_hash).await? {
                Some(res) => res,
                None => return Err(AppError::Unauthenticated),
            };

        if expires <= now || session_version != employee.version || !employee.active {
            return Err(AppError::Unauthenticated);
        }

        if employee.must_change && !allow_must_change {
            return Err(AppError::Domain(MUST_CHANGE.to_string()));
        }

        Ok(employee)
    }

    /// Ends the sessions of the tokens given. "" = none.
    pub async fn logout(&self, owner_token: &str, staff_token: &str) -> AppResult<()> {
        // Both deletes are attempted whatever the other did; a failure of either is the answer.
        let owner = if is_valid_token(owner_token) {
            self.repo
                .delete_owner_session(&sha256_hex(owner_token))
                .await
        } else {
            Ok(())
        };
        let staff = if is_valid_token(staff_token) {
            self.repo
                .delete_employee_session(&sha256_hex(staff_token))
                .await
        } else {
            Ok(())
        };
        owner.and(staff)
    }

    pub async fn change_password(
        &self,
        staff_token: &str,
        payload: &Value,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let employee = self.employee_session(staff_token, true, now).await?;

        let password_hash = self.hasher.hash(new_password(payload))?;
        let success = self
            .repo
            .change_employee_password(&employee.id, employee.version, &password_hash, now)
            .await?;
        if !success {
            return Err(AppError::Unauthenticated);
        }

        Ok(())
    }

    /// `action`: "list" | "create" | "reset" | "disable". `actor` is written to the audit trail as given.
    ///
    /// `disable` here takes no lock; `/api/employees` closes a found account through the
    /// `DisableEmployee` workflow instead, which asks custody again under the branch's lock.
    pub async fn employees(
        &self,
        action: &str,
        payload: &Value,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<EmployeesAnswer> {
        match action {
            "list" => {
                let list = self.repo.list_employees().await?;
                Ok(EmployeesAnswer::List(list))
            }
            "create" => {
                let username = match payload.get("username").and_then(|v| v.as_str()) {
                    Some(u) => u.trim().to_lowercase(),
                    None => return Err(AppError::Domain(EMPLOYEE_DATA.to_string())),
                };
                let is_valid_username = (3..=40).contains(&username.len())
                    && username.bytes().all(|b| {
                        b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || b == b'.'
                            || b == b'_'
                            || b == b'-'
                    });

                let name = match payload.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n.trim(),
                    None => return Err(AppError::Domain(EMPLOYEE_DATA.to_string())),
                };
                let name_utf16 = name.encode_utf16().count();
                let is_valid_name = (1..=100).contains(&name_utf16);

                let role = match payload.get("role").and_then(|v| v.as_str()) {
                    Some(r) => r,
                    None => return Err(AppError::Domain(EMPLOYEE_DATA.to_string())),
                };
                let is_valid_role = ROLES.contains(&role);

                let branch_id = match payload.get("branchId").and_then(|v| v.as_str()) {
                    Some(b) => b,
                    None => return Err(AppError::Domain(EMPLOYEE_DATA.to_string())),
                };
                let is_valid_branch = BranchId::is_well_formed(branch_id);

                if !is_valid_username || !is_valid_name || !is_valid_role || !is_valid_branch {
                    return Err(AppError::Domain(EMPLOYEE_DATA.to_string()));
                }

                let password_hash = self.hasher.hash(new_password(payload))?;
                let id = generate_uuid();
                let employee = Employee {
                    id,
                    username,
                    name: name.to_string(),
                    role: role.to_string(),
                    branch_id: branch_id.to_string(),
                    active: true,
                    must_change: true,
                    version: 1,
                    created_at: now,
                };
                self.repo
                    .create_employee(&employee, &password_hash, actor)
                    .await?;
                Ok(EmployeesAnswer::Saved)
            }
            "reset" | "disable" => {
                let id = match payload.get("id").and_then(|v| v.as_str()) {
                    Some(i) => i,
                    None => return Err(AppError::Domain(NOT_FOUND.to_string())),
                };
                let _ = self
                    .repo
                    .find_employee_by_id(id)
                    .await?
                    .ok_or_else(|| AppError::Domain(NOT_FOUND.to_string()))?;

                if action == "reset" {
                    let password_hash = self.hasher.hash(new_password(payload))?;
                    self.repo
                        .reset_employee_password(id, &password_hash, now, actor)
                        .await?;
                    Ok(EmployeesAnswer::Saved)
                } else {
                    self.repo.disable_employee(id, now, actor).await?;
                    Ok(EmployeesAnswer::Saved)
                }
            }
            _ => {
                let id = match payload.get("id").and_then(|v| v.as_str()) {
                    Some(i) => i,
                    None => return Err(AppError::Domain(NOT_FOUND.to_string())),
                };
                let _ = self
                    .repo
                    .find_employee_by_id(id)
                    .await?
                    .ok_or_else(|| AppError::Domain(NOT_FOUND.to_string()))?;
                Err(AppError::Domain(BAD_REQUEST.to_string()))
            }
        }
    }

    /// Closes the account of employee `id` inside `tx`, for a caller that holds
    /// a lock the closing must happen under (the route's `DisableEmployee`
    /// workflow, under the employee's branch lock). An id that names nobody is
    /// answered as `employees("disable")` answers it.
    pub async fn disable_employee_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &str,
        now: DateTime<Utc>,
        actor: &str,
    ) -> AppResult<()> {
        self.repo
            .find_employee_by_id(id)
            .await?
            .ok_or_else(|| AppError::Domain(NOT_FOUND.to_string()))?;
        self.repo.disable_employee_in(tx, id, now, actor).await
    }

    /// Deletes the sessions that have run out and the attempt counters of
    /// windows that are over, and says how many of each went. Run by the
    /// server on a timer; nothing a signed-in person can see changes.
    ///
    /// A session has run out when its `expires` is before `now`, with `now`
    /// cut to the millisecond as every stored time is. A window is over when
    /// it started before the window `now` falls in.
    pub async fn purge_expired(&self, now: DateTime<Utc>) -> AppResult<Purged> {
        let now = DateTime::from_timestamp_millis(now.timestamp_millis()).unwrap_or(now);
        self.repo.purge_expired(now, window_start(now)).await
    }

    /// Active employees of one branch (for T11a's receiver check and T11b's snapshot).
    pub async fn active_employees(&self, branch_id: &str) -> AppResult<Vec<Employee>> {
        self.repo.active_employees(branch_id).await
    }

    /// The one owner. Used by `create-owner` only.
    pub async fn create_owner(
        &self,
        username: &str,
        password: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Owner> {
        let trimmed_username = username.trim().to_lowercase();
        let is_valid_username = (3..=30).contains(&trimmed_username.len())
            && trimmed_username.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-'
            });
        if !is_valid_username {
            return Err(AppError::Domain("ชื่อบัญชีไม่ถูกต้อง".to_string()));
        }

        let password_hash = self.hasher.hash(password)?;
        let id = generate_uuid();
        let owner = self
            .repo
            .create_owner(
                &id,
                &trimmed_username,
                &trimmed_username,
                &password_hash,
                now,
            )
            .await?;
        Ok(owner)
    }
}
