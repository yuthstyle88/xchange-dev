use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

/// A verified token with extracted identity claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedToken {
    pub subject: String,
    pub expires_at: i64,
    pub phone: Option<String>,
}

/// Token verification errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("token malformed")]
    Malformed,
    #[error("bad signature")]
    BadSignature,
    #[error("wrong issuer")]
    WrongIssuer,
    #[error("wrong audience")]
    WrongAudience,
    #[error("token expired")]
    Expired,
    #[error("keys unavailable")]
    KeysUnavailable,
}

/// Trait for verifying compact JWT tokens.
pub trait TokenVerifier: Send + Sync {
    fn verify<'a>(&'a self, jwt: &'a str) -> BoxFuture<'a, Result<VerifiedToken, TokenError>>;
}
