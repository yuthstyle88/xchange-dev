//! Ticket T4c — the account service the server builds hashes with the cost for real accounts.
//!
//! `PasswordHasher::with_cost` is public (the tests of T4b use it to stay fast); the service
//! `auth::configure` builds must use `PasswordHasher::new()`. This holds an employee created
//! through `/api/employees` to the same check `create_owner_test` makes of the owner.
//! Needs `TEST_DATABASE_URL` like `auth_http_test`; it fails without it, it does not skip.

mod auth_http_common;
mod common;
