//! User domain module.

#[allow(clippy::module_inception)]
pub mod user;
pub mod user_repository;
pub mod user_status;

pub use user::User;
pub use user_repository::UserRepository;
pub use user_status::UserStatus;
