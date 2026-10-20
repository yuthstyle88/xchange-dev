//! RBAC §11.4: the PostgreSQL store of groups and their sets.
pub mod role_grant_store;

pub use role_grant_store::PgRoleGrantStore;
