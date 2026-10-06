//! Account service application layer.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

use crate::domain::account::entities::employee::Employee;
use crate::domain::account::entities::owner::Owner;
use crate::domain::account::entities::session::{Login, SignedIn};
use crate::domain::account::repositories::account_repository::AccountRepository;
use crate::domain::account::value_objects::password_hasher::PasswordHasher;

const WRONG: &str = "ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง";
const LIMIT: &str = "ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที";
const PASSWORD_LENGTH_MSG: &str = "รหัสผ่านต้องมี 12–128 ตัวอักษร";
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

const DUMMY_HASH: &str =
    "$argon2id$v=19$m=8,t=1,p=1$c2FsdHNhbHQ$dGVzdHRlc3R0ZXN0dGVzdHRlc3R0ZXN0dGVzdHRlc3Q";

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

#[derive(Clone)]
pub struct AccountService<R> {
    repo: Arc<R>,
    hasher: PasswordHasher,
}

impl<R: AccountRepository> AccountService<R> {
    pub fn new(repo: Arc<R>, hasher: PasswordHasher) -> Self {
        Self { repo, hasher }
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
                let password = payload.get("password").and_then(|v| v.as_str());
                let valid_pwd = if let Some(p) = password {
                    p.encode_utf16().count() <= 128 && self.hasher.verify(p, &password_hash)
                } else {
                    false
                };

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

        // (1) staff path only: a username longer than 40 -> WRONG, nothing counted
        if username.len() > 40 {
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
        let password = payload.get("password").and_then(|v| v.as_str());
        let emp_opt = if !username.is_empty() {
            self.repo.find_employee_by_username(&username).await?
        } else {
            None
        };

        let (employee, password_hash) = match emp_opt {
            Some(pair) => pair,
            None => {
                self.hasher.verify("dummy_password", DUMMY_HASH);
                return Err(AppError::Domain(WRONG.to_string()));
            }
        };

        if !employee.active {
            return Err(AppError::Domain(WRONG.to_string()));
        }

        let pwd = match password {
            Some(p) if p.encode_utf16().count() <= 128 => p,
            _ => return Err(AppError::Domain(WRONG.to_string())),
        };

        if !self.hasher.verify(pwd, &password_hash) {
            return Err(AppError::Domain(WRONG.to_string()));
        }

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
            match self.employee_session(staff_token, false, now).await {
                Ok(employee) => return Ok(Some(SignedIn::Employee(employee))),
                Err(_) => return Ok(None),
            }
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
        if !owner_token.is_empty() && is_valid_token(owner_token) {
            let token_hash = sha256_hex(owner_token);
            let _ = self.repo.delete_owner_session(&token_hash).await;
        }
        if !staff_token.is_empty() && is_valid_token(staff_token) {
            let token_hash = sha256_hex(staff_token);
            let _ = self.repo.delete_employee_session(&token_hash).await;
        }
        Ok(())
    }

    pub async fn change_password(
        &self,
        staff_token: &str,
        payload: &Value,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let employee = self.employee_session(staff_token, true, now).await?;

        let password = match payload.get("password").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string())),
        };

        let pwd_utf16 = password.encode_utf16().count();
        if !(12..=128).contains(&pwd_utf16) {
            return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string()));
        }

        let password_hash = self.hasher.hash(password)?;
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
                let is_valid_branch = (1..=70).contains(&branch_id.len())
                    && branch_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-');

                if !is_valid_username || !is_valid_name || !is_valid_role || !is_valid_branch {
                    return Err(AppError::Domain(EMPLOYEE_DATA.to_string()));
                }

                let password = match payload.get("password").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string())),
                };
                let pwd_utf16 = password.encode_utf16().count();
                if !(12..=128).contains(&pwd_utf16) {
                    return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string()));
                }

                let password_hash = self.hasher.hash(password)?;
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
                    let password = match payload.get("password").and_then(|v| v.as_str()) {
                        Some(p) => p,
                        None => return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string())),
                    };
                    let pwd_utf16 = password.encode_utf16().count();
                    if !(12..=128).contains(&pwd_utf16) {
                        return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string()));
                    }
                    let password_hash = self.hasher.hash(password)?;
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

        let pwd_utf16 = password.encode_utf16().count();
        if !(12..=128).contains(&pwd_utf16) {
            return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string()));
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
