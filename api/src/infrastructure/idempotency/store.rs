//! Idempotency store port and types (§6).

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use pos_error::AppResult;
use serde::{Deserialize, Serialize};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// An idempotency key identifying a unique command invocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for IdempotencyKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A token identifying a specific claim of an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClaimToken(String);

impl ClaimToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClaimToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for ClaimToken {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The result of claiming an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The key is new (or taken over) and was claimed with this token.
    New(ClaimToken),
    /// The key was previously finished with the same request hash; replays the stored response.
    Replay(serde_json::Value),
    /// The key was claimed with a different request hash, or another claim is in flight.
    Conflict,
}

/// Storage port for idempotency records.
pub trait IdempotencyStore: Send + Sync {
    /// Attempts to claim the idempotency key for the given operation and request hash.
    fn claim<'a>(
        &'a self,
        op: &'a str,
        key: &'a IdempotencyKey,
        hash: &'a str,
    ) -> BoxFuture<'a, AppResult<Claim>>;

    /// Finishes the idempotency record claimed by this token in the caller's transaction.
    ///
    /// Must update exactly the record `token` claimed while it is `PENDING`,
    /// returning `Err` otherwise so the caller's transaction rolls back.
    fn finish_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        op: &'a str,
        key: &'a IdempotencyKey,
        token: &'a ClaimToken,
        response: serde_json::Value,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Releases the idempotency record claimed by this token.
    fn release<'a>(
        &'a self,
        op: &'a str,
        key: &'a IdempotencyKey,
        token: &'a ClaimToken,
    ) -> BoxFuture<'a, AppResult<()>>;
}
