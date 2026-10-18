//! User domain module.

pub mod custody_holds;
#[allow(clippy::module_inception)]
pub mod user;
pub mod user_repository;
pub mod user_status;

pub use custody_holds::CustodyHolds;
pub use user::User;
pub use user_repository::UserRepository;
pub use user_status::UserStatus;
