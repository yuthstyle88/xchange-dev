//! User domain module.

pub mod custody_holds;
pub mod temp_password;
#[allow(clippy::module_inception)]
pub mod user;
pub mod user_history;
pub mod user_profile;
pub mod user_repository;
pub mod user_status;

pub use custody_holds::CustodyHolds;
pub use temp_password::TempPassword;
pub use user::User;
pub use user_history::{LoginEntry, UserAuditEntry};
pub use user_profile::UserProfile;
pub use user_repository::UserRepository;
pub use user_status::UserStatus;
