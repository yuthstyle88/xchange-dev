//! The user an `AccountAction` is aimed at (design §3.1).

use super::ScopedRole;
use crate::domain::user::UserStatus;

#[derive(Debug, Clone)]
pub struct TargetUser {
    pub id: String,
    pub status: UserStatus,
    /// Every row; the guard ignores roles that are not live.
    pub roles: Vec<ScopedRole>,
    pub has_auth_subject: bool,
}
