//! `LoyaltyConfig` (design §6, rulings 1): settings for delivering loyalty outbox events.

use std::time::Duration;
use uuid::Uuid;

/// Destination settings for Loyalty event delivery.
#[derive(Clone, PartialEq, Eq)]
pub struct LoyaltyConfig {
    pub base_url: String,
    pub tenant_id: Uuid,
    pub webhook_secret: String,
    pub timeout: Duration,
    pub service_token: Option<String>,
}

impl LoyaltyConfig {
    /// Builds `LoyaltyConfig` from optional environment strings.
    /// Returns `None` if any of `base_url`, `tenant_id`, or `webhook_secret` is unset, blank,
    /// or if `tenant_id` is not a valid UUID.
    /// `timeout_ms` defaults to 5000 ms if unset, blank, or invalid.
    /// `service_token` is optional (trimmed; `None` if unset or blank).
    pub fn from_raw(
        base_url: Option<&str>,
        tenant_id: Option<&str>,
        webhook_secret: Option<&str>,
        timeout_ms: Option<&str>,
        service_token: Option<&str>,
    ) -> Option<Self> {
        let base_url = base_url?.trim();
        if base_url.is_empty() {
            return None;
        }

        let tenant_raw = tenant_id?.trim();
        if tenant_raw.is_empty() {
            return None;
        }
        let tenant_id = Uuid::parse_str(tenant_raw).ok()?;

        let webhook_secret = webhook_secret?.trim();
        if webhook_secret.is_empty() {
            return None;
        }

        let timeout = timeout_ms
            .and_then(|ms| ms.trim().parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(5000));

        let service_token = service_token
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        Some(Self {
            base_url: base_url.to_owned(),
            tenant_id,
            webhook_secret: webhook_secret.to_owned(),
            timeout,
            service_token,
        })
    }
}

/// Never prints the webhook secret or service token.
impl std::fmt::Debug for LoyaltyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoyaltyConfig")
            .field("base_url", &self.base_url)
            .field("tenant_id", &self.tenant_id)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}
