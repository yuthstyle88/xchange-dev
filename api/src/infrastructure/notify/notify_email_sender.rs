//! `NotifyEmailSender` (design `kiosk-shared-phone.md` §4.2 D21, §4.1 "E-mail"): Notify
//! `POST /api/v1/notifications`, `channel: "email"`, header `X-API-Key`, beside
//! `NotifySmsSender`. Off when `NOTIFY_BASE_URL` or `NOTIFY_API_KEY` is unset.
//!
//! Notify's `CreateNotificationRequest` (`108-platform-services`
//! `crates/notification-api/src/dto.rs:17-51`): `{channel, recipient: {address, display_name?},
//! subject?, body?, html_body?, idempotency_key?, …}`; it answers `201` when created and `200` when
//! the idempotency key matched (`routes/notifications.rs:46-62`). It has no attachment field.

use std::time::Duration;

use pos_error::{AppError, AppResult};
use xc_customer::domain::email_sender::{
    EMAIL_UNAVAILABLE, EmailMessage, EmailOutcome, EmailSender,
};
use xc_customer::domain::sms_sender::BoxFuture;

#[derive(Clone, Debug)]
pub struct NotifyEmailSender {
    base_url: Option<String>,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl NotifyEmailSender {
    pub fn new(base_url: Option<String>, api_key: Option<String>) -> Self {
        let client = crate::infrastructure::http_client::build_client(
            Duration::from_secs(10),
            "xchange-notify",
        );
        Self {
            base_url,
            api_key,
            client,
        }
    }

    pub fn from_env() -> Self {
        Self::new(
            std::env::var("NOTIFY_BASE_URL").ok(),
            std::env::var("NOTIFY_API_KEY").ok(),
        )
    }

    /// The JSON body Notify receives for `message`: exactly `channel` (`"email"`),
    /// `recipient.address`, `subject`, `body` (the text), `html_body` and `idempotency_key`.
    pub fn payload(message: &EmailMessage) -> serde_json::Value {
        serde_json::json!({
            "channel": "email",
            "recipient": { "address": message.to },
            "subject": message.subject,
            "body": message.text,
            "html_body": message.html,
            "idempotency_key": message.idempotency_key,
        })
    }
}

impl EmailSender for NotifyEmailSender {
    fn is_configured(&self) -> bool {
        self.base_url
            .as_deref()
            .is_some_and(|u| !u.trim().is_empty())
            && self
                .api_key
                .as_deref()
                .is_some_and(|k| !k.trim().is_empty())
    }

    fn send<'a>(&'a self, message: &'a EmailMessage) -> BoxFuture<'a, AppResult<EmailOutcome>> {
        Box::pin(async move {
            if !self.is_configured() {
                return Err(AppError::Domain(EMAIL_UNAVAILABLE.to_owned()));
            }
            let base_url = self
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .trim_end_matches('/');
            let api_key = self.api_key.as_deref().unwrap_or_default().trim();
            let endpoint = format!("{base_url}/api/v1/notifications");
            let resp = self
                .client
                .post(&endpoint)
                .header("x-api-key", api_key)
                .json(&Self::payload(message))
                .send()
                .await
                .map_err(|_| AppError::Domain(EMAIL_UNAVAILABLE.to_owned()))?;
            match resp.status().as_u16() {
                201 => Ok(EmailOutcome::Sent),
                200 => Ok(EmailOutcome::AlreadySent),
                _ => Err(AppError::Domain(EMAIL_UNAVAILABLE.to_owned())),
            }
        })
    }
}
