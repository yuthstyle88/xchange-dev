//! `NotifyEmailSender` (design `kiosk-shared-phone.md` §4.2 D21, §4.1 "E-mail"): Notify
//! `POST /api/v1/notifications`, `channel: "email"`, header `X-API-Key`, beside
//! `NotifySmsSender`. Off when `NOTIFY_BASE_URL` or `NOTIFY_API_KEY` is unset.
//!
//! Notify's `CreateNotificationRequest` (`108-platform-services`
//! `crates/notification-api/src/dto.rs:17-51`): `{channel, recipient: {address, display_name?},
//! subject?, body?, html_body?, idempotency_key?, …}`; it answers `201` when created and `200` when
//! the idempotency key matched (`routes/notifications.rs:46-62`). It has no attachment field.

use std::time::Duration;

use pos_error::AppResult;
use xc_customer::domain::email_sender::{EmailMessage, EmailOutcome, EmailSender};
use xc_customer::domain::sms_sender::BoxFuture;

#[derive(Clone, Debug)]
pub struct NotifyEmailSender {
    base_url: Option<String>,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl NotifyEmailSender {
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
        Self::new(
            std::env::var("NOTIFY_BASE_URL").ok(),
            std::env::var("NOTIFY_API_KEY").ok(),
        )
    }

    /// The JSON body Notify receives for `message`: exactly `channel` (`"email"`),
    /// `recipient.address`, `subject`, `body` (the text), `html_body` and `idempotency_key`.
    pub fn payload(message: &EmailMessage) -> serde_json::Value {
        let _ = message;
        todo!("KWQ-D1: Notify's CreateNotificationRequest field names")
    }
}

impl EmailSender for NotifyEmailSender {
    fn is_configured(&self) -> bool {
        let _ = (&self.base_url, &self.api_key, &self.client);
        todo!("KWQ-D1: both NOTIFY_BASE_URL and NOTIFY_API_KEY set and non-empty")
    }

    fn send<'a>(&'a self, message: &'a EmailMessage) -> BoxFuture<'a, AppResult<EmailOutcome>> {
        let _ = message;
        Box::pin(
            async move { todo!("KWQ-D1: POST /api/v1/notifications; 201 Sent, 200 AlreadySent") },
        )
    }
}
