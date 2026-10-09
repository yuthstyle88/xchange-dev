use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use super::token_verifier::TokenError;

/// A JSON Web Key for Ed25519 (OKP) keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwk {
    pub kid: String,
    pub kty: String,
    pub crv: String,
    pub x: String,
}

/// A set of JSON Web Keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

/// Source for fetching JWKS keys.
pub trait JwksSource: Send + Sync {
    fn fetch(&self) -> BoxFuture<'_, Result<Jwks, TokenError>>;
}
