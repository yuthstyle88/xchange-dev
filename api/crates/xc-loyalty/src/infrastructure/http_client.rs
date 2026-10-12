//! `LoyaltyHttp` (design §6, D-3): signed HTTP client to Loyalty's webhook endpoint.

use uuid::Uuid;

use crate::infrastructure::config::LoyaltyConfig;

#[derive(Clone)]
pub struct LoyaltyHttp {
    client: reqwest::Client,
    base_url: String,
    tenant_id: Uuid,
    webhook_secret: String,
}

impl LoyaltyHttp {
    pub fn new(config: &LoyaltyConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("valid reqwest client");
        Self {
            client,
            base_url: config.base_url.clone(),
            tenant_id: config.tenant_id,
            webhook_secret: config.webhook_secret.clone(),
        }
    }

    /// Sign the body string using `X-POS108-Signature: t=<timestamp>,v1=<hex>`.
    pub fn sign(&self, timestamp: i64, body: &str) -> String {
        pos_kernel::webhook_signature::signature_header(
            &self.webhook_secret,
            &timestamp.to_string(),
            body.as_bytes(),
        )
    }

    /// Delivers one event payload to Loyalty's `/webhooks/xchange`.
    pub async fn post_event(
        &self,
        _event_id: Uuid,
        idempotency_key: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::StatusCode, reqwest::Error> {
        let body_str = serde_json::to_string(body).unwrap_or_default();
        let ts = chrono::Utc::now().timestamp();
        let sig = self.sign(ts, &body_str);
        let url = format!("{}/webhooks/xchange", self.base_url.trim_end_matches('/'));

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", idempotency_key)
            .header(pos_kernel::webhook_signature::HEADER, sig)
            .body(body_str)
            .send()
            .await?;

        Ok(resp.status())
    }
}

/// Never prints the webhook secret.
impl std::fmt::Debug for LoyaltyHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoyaltyHttp")
            .field("base_url", &self.base_url)
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}
