//! Notify SMS sender adapter (CU-P1, design §6.1, §6.3).

use std::time::Duration;

use pos_error::{AppError, AppResult};
use xc_customer::domain::sms_sender::{BoxFuture, SmsSender};

#[derive(Clone, Debug)]
pub struct NotifySmsSender {
    base_url: Option<String>,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl NotifySmsSender {
    pub fn new(base_url: Option<String>, api_key: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url,
            api_key,
            client,
        }
    }

    pub fn from_env() -> Self {
        let base_url = std::env::var("NOTIFY_BASE_URL").ok();
        let api_key = std::env::var("NOTIFY_API_KEY").ok();
        Self::new(base_url, api_key)
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
}

impl SmsSender for NotifySmsSender {
    fn send<'a>(&'a self, phone: &'a str, text: &'a str) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let base_url = self.base_url.as_deref().ok_or_else(|| {
                AppError::Domain("smsUnavailable: NOTIFY_BASE_URL is not configured".to_string())
            })?;
            if base_url.trim().is_empty() {
                return Err(AppError::Domain(
                    "smsUnavailable: NOTIFY_BASE_URL is empty".to_string(),
                ));
            }

            let endpoint = format!("{}/api/v1/notifications", base_url.trim_end_matches('/'));
            let payload = serde_json::json!({
                "channel": "sms",
                "phone": phone,
                "text": text,
            });

            let mut req = self.client.post(&endpoint).json(&payload);
            if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
                req = req.header("x-api-key", key);
            }

            let resp = req
                .send()
                .await
                .map_err(|e| AppError::Domain(format!("smsUnavailable: network error: {e}")))?;

            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                Err(AppError::Domain(format!(
                    "smsUnavailable: server responded with status {status}"
                )))
            }
        })
    }
}
