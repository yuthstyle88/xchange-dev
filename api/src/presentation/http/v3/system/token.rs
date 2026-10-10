//! The shared secret Broker sends as `X-System-Token` when it pulls the exchange feed (FD-3 #974).
//!
//! Scaffold written by the ticket author: the bodies are `todo!()`; the implementer fills them.

use subtle::ConstantTimeEq;

/// The environment variable holding the feed token. It is not `XC_BROKER_SERVICE_TOKEN`, the
/// outbound one: two secrets for two directions (design §5.4, §7).
pub const FEED_TOKEN_ENV: &str = "XC_BROKER_FEED_TOKEN";

/// The header Broker sends.
pub const HEADER: &str = "x-system-token";

/// The configured token, read once at boot. Unset or blank ⇒ every request is refused.
#[derive(Debug, Clone)]
pub struct SystemToken {
    expected: Option<Vec<u8>>,
}

impl SystemToken {
    /// From a configured value: `None`, empty or whitespace-only ⇒ accepts nothing.
    pub fn from_value(value: Option<String>) -> Self {
        let expected = value.and_then(|v| {
            if v.trim().is_empty() {
                None
            } else {
                Some(v.into_bytes())
            }
        });
        Self { expected }
    }

    /// Reads [`FEED_TOKEN_ENV`] once.
    pub fn from_env() -> Self {
        Self::from_value(std::env::var(FEED_TOKEN_ENV).ok())
    }

    /// Whether `presented` (the header's bytes, `None` when absent) is the configured token,
    /// compared in constant time (`subtle::ConstantTimeEq`).
    pub fn accepts(&self, presented: Option<&[u8]>) -> bool {
        match (presented, &self.expected) {
            (Some(presented), Some(expected)) => bool::from(presented.ct_eq(expected.as_slice())),
            _ => false,
        }
    }
}
