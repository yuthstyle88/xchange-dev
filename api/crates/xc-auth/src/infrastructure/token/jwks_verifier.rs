use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use futures::future::BoxFuture;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::Deserialize;

use crate::domain::token::{Jwk, Jwks, JwksSource, TokenError, TokenVerifier, VerifiedToken};

/// Default minimum interval between JWKS refetches (60 seconds).
pub const DEFAULT_REFETCH_INTERVAL: Duration = Duration::from_secs(60);

const CLOCK_LEEWAY_SECS: i64 = 60;

/// Monotonic clock for measuring refetch intervals.
pub trait RefetchClock: Send + Sync + 'static {
    fn now(&self) -> Duration;
}

/// System monotonic clock backed by `Instant`.
#[derive(Clone, Debug)]
pub struct SystemRefetchClock {
    epoch: Instant,
}

impl Default for SystemRefetchClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl RefetchClock for SystemRefetchClock {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }
}

struct CacheState {
    jwks: Option<Jwks>,
    last_fetch: Option<Duration>,
}

#[derive(Deserialize)]
struct JwtHeader {
    alg: Option<String>,
    kid: Option<String>,
}

fn decode_b64(input: &str) -> Result<Vec<u8>, TokenError> {
    URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| URL_SAFE.decode(input))
        .map_err(|_| TokenError::Malformed)
}

fn find_valid_key<'a>(jwks: &'a Jwks, kid: &str) -> Option<&'a Jwk> {
    jwks.keys
        .iter()
        .find(|k| k.kid == kid && k.kty == "OKP" && k.crv == "Ed25519")
}

/// Token verifier using keys fetched from a `JwksSource`.
#[derive(Clone)]
pub struct JwksTokenVerifier {
    source: Arc<dyn JwksSource>,
    issuer: String,
    audience: String,
    min_interval: Duration,
    clock: Arc<dyn RefetchClock>,
    cache: Arc<RwLock<CacheState>>,
    flight: Arc<futures::lock::Mutex<()>>,
}

impl JwksTokenVerifier {
    pub fn new(
        source: Arc<dyn JwksSource>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self::with_refetch(
            source,
            issuer,
            audience,
            DEFAULT_REFETCH_INTERVAL,
            Arc::new(SystemRefetchClock::default()),
        )
    }

    pub fn with_refetch(
        source: Arc<dyn JwksSource>,
        issuer: impl Into<String>,
        audience: impl Into<String>,
        min_interval: Duration,
        clock: Arc<dyn RefetchClock>,
    ) -> Self {
        Self {
            source,
            issuer: issuer.into(),
            audience: audience.into(),
            min_interval,
            clock,
            cache: Arc::new(RwLock::new(CacheState {
                jwks: None,
                last_fetch: None,
            })),
            flight: Arc::new(futures::lock::Mutex::new(())),
        }
    }

    async fn resolve_key(&self, kid: &str) -> Result<Jwk, TokenError> {
        // Fast path: check cache under read lock
        {
            let guard = self.cache.read().map_err(|_| TokenError::KeysUnavailable)?;
            if let Some(ref jwks) = guard.jwks {
                if let Some(key) = find_valid_key(jwks, kid) {
                    return Ok(key.clone());
                }
            }
        }

        // Single-flight lock: at most one refetch in flight at a time
        let _flight_guard = self.flight.lock().await;

        // Double check cache after acquiring flight lock
        {
            let guard = self.cache.read().map_err(|_| TokenError::KeysUnavailable)?;
            if let Some(ref jwks) = guard.jwks {
                if let Some(key) = find_valid_key(jwks, kid) {
                    return Ok(key.clone());
                }
                // Cache is loaded, but kid is not present: check rate limit window
                let now = self.clock.now();
                if let Some(last) = guard.last_fetch {
                    if now.saturating_sub(last) < self.min_interval {
                        return Err(TokenError::BadSignature);
                    }
                }
            }
        }

        // Fetch keys from source
        let fetch_time = self.clock.now();
        let fetched = self.source.fetch().await?;
        let maybe_key = find_valid_key(&fetched, kid).cloned();

        {
            let mut guard = self
                .cache
                .write()
                .map_err(|_| TokenError::KeysUnavailable)?;
            guard.jwks = Some(fetched);
            guard.last_fetch = Some(fetch_time);
        }

        maybe_key.ok_or(TokenError::BadSignature)
    }

    async fn verify_jwt(&self, jwt: &str) -> Result<VerifiedToken, TokenError> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() != 3 {
            return Err(TokenError::Malformed);
        }

        let header_bytes = decode_b64(parts[0])?;
        let header: JwtHeader =
            serde_json::from_slice(&header_bytes).map_err(|_| TokenError::Malformed)?;

        if header.alg.as_deref() != Some("EdDSA") {
            return Err(TokenError::Malformed);
        }

        let kid = match header.kid.as_deref() {
            Some(k) if !k.is_empty() => k,
            _ => return Err(TokenError::Malformed),
        };

        let payload_bytes = decode_b64(parts[1])?;
        let payload: serde_json::Value =
            serde_json::from_slice(&payload_bytes).map_err(|_| TokenError::Malformed)?;
        let payload_obj = payload.as_object().ok_or(TokenError::Malformed)?;

        let sub = payload_obj
            .get("sub")
            .and_then(|v| v.as_str())
            .ok_or(TokenError::Malformed)?;

        let exp = payload_obj
            .get("exp")
            .and_then(|v| v.as_i64())
            .ok_or(TokenError::Malformed)?;

        let phone = match payload_obj.get("phone") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            _ => return Err(TokenError::Malformed),
        };

        let sig_bytes = decode_b64(parts[2])?;

        let jwk = self.resolve_key(kid).await?;
        let pubkey_bytes = URL_SAFE_NO_PAD
            .decode(&jwk.x)
            .or_else(|_| URL_SAFE.decode(&jwk.x))
            .map_err(|_| TokenError::BadSignature)?;

        let public_key = UnparsedPublicKey::new(&ED25519, &pubkey_bytes);
        let signed_data = format!("{}.{}", parts[0], parts[1]);
        public_key
            .verify(signed_data.as_bytes(), &sig_bytes)
            .map_err(|_| TokenError::BadSignature)?;

        match payload_obj.get("iss").and_then(|v| v.as_str()) {
            Some(iss) if iss == self.issuer => {}
            _ => return Err(TokenError::WrongIssuer),
        }

        let aud_valid = match payload_obj.get("aud") {
            Some(serde_json::Value::String(s)) => s == &self.audience,
            Some(serde_json::Value::Array(arr)) => {
                arr.iter().any(|v| v.as_str() == Some(&self.audience))
            }
            _ => false,
        };
        if !aud_valid {
            return Err(TokenError::WrongAudience);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        if exp.saturating_add(CLOCK_LEEWAY_SECS) < now {
            return Err(TokenError::Expired);
        }

        Ok(VerifiedToken {
            subject: sub.to_string(),
            expires_at: exp,
            phone,
        })
    }
}

impl TokenVerifier for JwksTokenVerifier {
    fn verify<'a>(&'a self, jwt: &'a str) -> BoxFuture<'a, Result<VerifiedToken, TokenError>> {
        Box::pin(async move { self.verify_jwt(jwt).await })
    }
}
